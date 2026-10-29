//! Local accounts, password hashing, sessions and mandatory API authorization.
use super::*;
use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{
    body::{to_bytes, Body},
    http::Method,
    response::Redirect,
};
use rand::{rngs::OsRng, RngCore};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const SESSION_SECONDS: i64 = 8 * 60 * 60;
const AUTH_COOKIE: &str = "parcours_session";
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Principal {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role: String,
    pub subject_id: Option<String>,
    pub patient_ids: Vec<String>,
    pub enabled: bool,
    pub activated: bool,
}
impl Principal {
    pub fn home(&self) -> &str {
        match self.role.as_str() {
            "admin" => "/admin",
            "patient" => "/patient",
            "nurse" => "/domicile",
            "pharmacy" => "/pharmacie",
            _ => "/",
        }
    }
    fn has_patient(&self, id: &str) -> bool {
        self.patient_ids.iter().any(|p| p == id)
    }
}
#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}
#[derive(Deserialize)]
pub struct Setup {
    username: String,
    name: String,
    password: String,
}
#[derive(Deserialize)]
pub struct Activation {
    code: String,
    password: String,
}
#[derive(Deserialize)]
pub struct Invitation {
    username: String,
    name: String,
    role: String,
    subject_id: Option<String>,
    #[serde(default)]
    patient_ids: Vec<String>,
}
#[derive(Deserialize)]
pub struct AccessUpdate {
    subject_id: Option<String>,
    #[serde(default)]
    patient_ids: Vec<String>,
    enabled: bool,
}
#[derive(Deserialize)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}
#[derive(Deserialize)]
pub struct Reissue {
    account_id: String,
}
fn forbidden() -> ApiError {
    ApiError(
        StatusCode::FORBIDDEN,
        "Vous n’avez pas accès à cette action ou à ce dossier.".into(),
    )
}
fn unauthorized() -> ApiError {
    ApiError(
        StatusCode::UNAUTHORIZED,
        "Connectez-vous pour accéder à votre espace.".into(),
    )
}
fn auth_fail() -> ApiError {
    ApiError(
        StatusCode::UNAUTHORIZED,
        "Identifiant ou mot de passe incorrect, ou compte indisponible.".into(),
    )
}
fn secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn username(s: &str) -> Result<String> {
    let s = s.trim().to_ascii_lowercase();
    if !(3..=64).contains(&s.len())
        || !s
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
    {
        return Err(invalid("L’identifiant doit contenir 3 à 64 lettres non accentuées, chiffres, points, tirets ou tirets bas."));
    }
    Ok(s)
}
fn password(s: &str) -> Result<()> {
    if !(12..=128).contains(&s.chars().count()) || s.len() > 512 {
        return Err(invalid(
            "Choisissez un mot de passe de 12 à 128 caractères.",
        ));
    }
    Ok(())
}
fn hash_password(s: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(s.as_bytes(), &salt)
        .map(|p| p.to_string())
        .map_err(internal)
}
fn verify(s: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|p| Argon2::default().verify_password(s.as_bytes(), &p).is_ok())
        .unwrap_or(false)
}
async fn hash_async(s: String) -> Result<String> {
    password(&s)?;
    tokio::task::spawn_blocking(move || hash_password(&s))
        .await
        .map_err(internal)?
}
pub fn init(db: &Connection) -> std::result::Result<(), rusqlite::Error> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS auth_accounts(id TEXT PRIMARY KEY,username TEXT NOT NULL UNIQUE,name TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('admin','doctor','team','patient','nurse','pharmacy')),subject_id TEXT,patient_ids TEXT NOT NULL DEFAULT '[]',password_hash TEXT,enabled INTEGER NOT NULL DEFAULT 1,activation_hash TEXT UNIQUE,activation_expires INTEGER,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS auth_sessions(token_hash TEXT PRIMARY KEY,account_id TEXT NOT NULL REFERENCES auth_accounts(id),expires_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS auth_attempts(username TEXT PRIMARY KEY,failures INTEGER NOT NULL,locked_until INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS auth_limits(key TEXT PRIMARY KEY,window INTEGER NOT NULL,count INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS auth_audit(id TEXT PRIMARY KEY,at TEXT NOT NULL,account_id TEXT,action TEXT NOT NULL);
")
}
fn log(db: &Connection, account: Option<&str>, action: &str) -> Result<()> {
    db.execute(
        "INSERT INTO auth_audit VALUES(?1,?2,?3,?4)",
        params![id(), Utc::now().to_rfc3339(), account, action],
    )
    .map_err(internal)?;
    Ok(())
}
fn principal(db: &Connection, aid: &str) -> Result<Principal> {
    db.query_row("SELECT id,username,name,role,subject_id,patient_ids,enabled,password_hash IS NOT NULL FROM auth_accounts WHERE id=?1",[aid],|r|{let ids:String=r.get(5)?;Ok(Principal{id:r.get(0)?,username:r.get(1)?,name:r.get(2)?,role:r.get(3)?,subject_id:r.get(4)?,patient_ids:serde_json::from_str(&ids).map_err(|e|rusqlite::Error::FromSqlConversionFailure(5,rusqlite::types::Type::Text,Box::new(e)))?,enabled:r.get(6)?,activated:r.get(7)?})}).map_err(internal)
}
fn cookie_token(req: &Request) -> Option<String> {
    req.headers()
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|s| {
            s.trim()
                .strip_prefix(&format!("{AUTH_COOKIE}="))
                .map(str::to_owned)
        })
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn session(db: &Connection, token: &str) -> Result<Principal> {
    let aid:Option<String>=db.query_row("SELECT s.account_id FROM auth_sessions s JOIN auth_accounts a ON a.id=s.account_id WHERE s.token_hash=?1 AND s.expires_at>?2 AND a.enabled=1 AND a.password_hash IS NOT NULL",params![digest(token),Utc::now().timestamp()],|r|r.get(0)).optional().map_err(internal)?;
    let p = principal(db, &aid.ok_or_else(unauthorized)?)?;
    Ok(p)
}
fn create_session(db: &Connection, aid: &str) -> Result<String> {
    let token = secret();
    db.execute(
        "DELETE FROM auth_sessions WHERE expires_at<=?1",
        [Utc::now().timestamp()],
    )
    .map_err(internal)?;
    db.execute(
        "INSERT INTO auth_sessions VALUES(?1,?2,?3)",
        params![
            digest(&token),
            aid,
            Utc::now().timestamp() + SESSION_SECONDS
        ],
    )
    .map_err(internal)?;
    Ok(token)
}
fn secure_cookie_suffix() -> &'static str {
    if matches!(
        std::env::var("PARCOURS_SECURE_COOKIES").as_deref(),
        Ok("true") | Ok("1")
    ) {
        "; Secure"
    } else {
        ""
    }
}
fn signed_in(p: Principal, token: String) -> Response {
    let home = p.home().to_string();
    let mut r = Json(serde_json::json!({"user":p,"home":home})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{AUTH_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_SECONDS}{}",
            secure_cookie_suffix()
        )
        .parse()
        .unwrap(),
    );
    r
}
fn binding(d: &Data, role: &str, subject: &Option<String>, ids: &[String]) -> Result<()> {
    let valid = match role {
        "admin" => subject.is_none() && ids.is_empty(),
        "doctor" | "team" => {
            subject.is_none() && ids.iter().all(|id| d.patients.iter().any(|p| &p.id == id))
        }
        "patient" => {
            ids.is_empty()
                && subject
                    .as_ref()
                    .map(|s| d.patients.iter().any(|p| &p.id == s))
                    .unwrap_or(false)
        }
        "nurse" => {
            ids.is_empty()
                && subject
                    .as_ref()
                    .map(|s| d.nurses.iter().any(|p| &p.id == s))
                    .unwrap_or(false)
        }
        "pharmacy" => {
            ids.is_empty()
                && subject
                    .as_ref()
                    .map(|s| d.pharmacies.iter().any(|p| &p.id == s))
                    .unwrap_or(false)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("Rôle ou affectation invalide. Choisissez des dossiers et un profil existants correspondant au rôle."))
    }
}
fn must_admin(p: &Principal) -> Result<()> {
    if p.role == "admin" {
        Ok(())
    } else {
        Err(forbidden())
    }
}
pub async fn status(State(app): State<App>) -> Result<Json<serde_json::Value>> {
    let db = app.db.lock().map_err(internal)?;
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM auth_accounts", [], |r| r.get(0))
        .map_err(internal)?;
    Ok(Json(serde_json::json!({"setup_required":count==0})))
}
pub async fn setup(State(app): State<App>, Json(i): Json<Setup>) -> Result<Response> {
    let name = i.name.trim().to_string();
    required(&name, "Nom", 120)?;
    let uname = username(&i.username)?;
    {
        let db = app.db.lock().map_err(internal)?;
        let n: i64 = db
            .query_row("SELECT COUNT(*) FROM auth_accounts", [], |r| r.get(0))
            .map_err(internal)?;
        if n != 0 {
            return Err(forbidden());
        }
    }
    let hash = hash_async(i.password).await?;
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    let n: i64 = tx
        .query_row("SELECT COUNT(*) FROM auth_accounts", [], |r| r.get(0))
        .map_err(internal)?;
    if n != 0 {
        return Err(forbidden());
    }
    let aid = id();
    tx.execute("INSERT INTO auth_accounts(id,username,name,role,password_hash,created_at) VALUES(?1,?2,?3,'admin',?4,?5)",params![aid,uname,name,hash,Utc::now().to_rfc3339()]).map_err(internal)?;
    log(&tx, Some(&aid), "Premier compte administrateur créé")?;
    let p = principal(&tx, &aid)?;
    let token = create_session(&tx, &aid)?;
    tx.commit().map_err(internal)?;
    Ok(signed_in(p, token))
}
pub async fn login(State(app): State<App>, Json(i): Json<Credentials>) -> Result<Response> {
    let uname = i.username.trim().to_ascii_lowercase();
    if uname.len() > 64 || i.password.len() > 512 {
        return Err(auth_fail());
    }
    let aid_hash = {
        let db = app.db.lock().map_err(internal)?;
        let now = Utc::now().timestamp();
        let window = now / 60;
        db.execute("INSERT INTO auth_limits VALUES('login',?1,1) ON CONFLICT(key) DO UPDATE SET count=CASE WHEN window=excluded.window THEN count+1 ELSE 1 END,window=excluded.window",[window]).map_err(internal)?;
        let count: i64 = db
            .query_row("SELECT count FROM auth_limits WHERE key='login'", [], |r| {
                r.get(0)
            })
            .map_err(internal)?;
        let locked: i64 = db
            .query_row(
                "SELECT locked_until FROM auth_attempts WHERE username=?1",
                [&uname],
                |r| r.get(0),
            )
            .optional()
            .map_err(internal)?
            .unwrap_or(0);
        if count > 30 || locked > now {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Trop de tentatives. Réessayez dans quelques minutes.".into(),
            ));
        }
        db.query_row("SELECT id,password_hash FROM auth_accounts WHERE username=?1 AND enabled=1 AND password_hash IS NOT NULL",[&uname],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(internal)?
    };
    let candidate = i.password;
    let stored = aid_hash.as_ref().map(|(_, h)| h.clone());
    let valid = tokio::task::spawn_blocking(move || {
        if let Some(h) = stored {
            verify(&candidate, &h)
        } else {
            let _ = hash_password(&candidate);
            false
        }
    })
    .await
    .map_err(internal)?;
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    if !valid {
        let now = Utc::now().timestamp();
        tx.execute("INSERT INTO auth_attempts VALUES(?1,1,0) ON CONFLICT(username) DO UPDATE SET failures=CASE WHEN locked_until>0 AND locked_until<=?2 THEN 1 ELSE failures+1 END,locked_until=CASE WHEN failures>=4 AND (locked_until=0 OR locked_until>?2) THEN ?2+300 ELSE 0 END",params![uname,now]).map_err(internal)?;
        log(&tx, None, "Échec de connexion")?;
        tx.commit().map_err(internal)?;
        return Err(auth_fail());
    }
    let (aid, verified_hash) = aid_hash.ok_or_else(auth_fail)?;
    let current_hash: Option<String> = tx
        .query_row(
            "SELECT password_hash FROM auth_accounts WHERE id=?1 AND enabled=1",
            [&aid],
            |r| r.get(0),
        )
        .optional()
        .map_err(internal)?
        .flatten();
    if current_hash.as_ref() != Some(&verified_hash) {
        return Err(auth_fail());
    }
    tx.execute("DELETE FROM auth_attempts WHERE username=?1", [&uname])
        .map_err(internal)?;
    let p = principal(&tx, &aid)?;
    let token = create_session(&tx, &aid)?;
    log(&tx, Some(&aid), "Connexion réussie")?;
    tx.commit().map_err(internal)?;
    Ok(signed_in(p, token))
}
pub async fn activate(State(app): State<App>, Json(i): Json<Activation>) -> Result<Response> {
    if i.code.len() != 64 {
        return Err(invalid("Code d’activation invalide ou expiré."));
    }
    let code_hash = digest(i.code.trim());
    {
        let db = app.db.lock().map_err(internal)?;
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM auth_accounts WHERE activation_hash=?1 AND activation_expires>?2 AND enabled=1 AND password_hash IS NULL)",params![code_hash,Utc::now().timestamp()],|r|r.get(0)).map_err(internal)?;
        if !valid {
            return Err(invalid("Code d’activation invalide ou expiré."));
        }
    }
    let hash = hash_async(i.password).await?;
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    let aid:Option<String>=tx.query_row("SELECT id FROM auth_accounts WHERE activation_hash=?1 AND activation_expires>?2 AND enabled=1 AND password_hash IS NULL",params![code_hash,Utc::now().timestamp()],|r|r.get(0)).optional().map_err(internal)?;
    let aid = aid.ok_or(invalid("Code d’activation invalide ou expiré."))?;
    tx.execute("UPDATE auth_accounts SET password_hash=?1,activation_hash=NULL,activation_expires=NULL WHERE id=?2",params![hash,aid]).map_err(internal)?;
    let p = principal(&tx, &aid)?;
    let token = create_session(&tx, &aid)?;
    log(&tx, Some(&aid), "Compte activé")?;
    tx.commit().map_err(internal)?;
    Ok(signed_in(p, token))
}
pub async fn me(axum::Extension(p): axum::Extension<Principal>) -> Json<serde_json::Value> {
    let home = p.home().to_string();
    Json(serde_json::json!({"user":p,"home":home}))
}
pub async fn logout(State(app): State<App>, req: Request) -> Result<Response> {
    if let Some(token) = cookie_token(&req) {
        let db = app.db.lock().map_err(internal)?;
        db.execute(
            "DELETE FROM auth_sessions WHERE token_hash=?1",
            [digest(&token)],
        )
        .map_err(internal)?;
    }
    let mut r = Json(serde_json::json!({"ok":true})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{AUTH_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
            secure_cookie_suffix()
        )
        .parse()
        .unwrap(),
    );
    Ok(r)
}
pub async fn change_password(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(i): Json<PasswordChange>,
) -> Result<Response> {
    let old_hash: String = {
        let db = app.db.lock().map_err(internal)?;
        db.query_row(
            "SELECT password_hash FROM auth_accounts WHERE id=?1",
            [&p.id],
            |r| r.get(0),
        )
        .map_err(internal)?
    };
    if i.current_password.len() > 512 {
        return Err(invalid("Mot de passe actuel incorrect."));
    }
    let test_hash = old_hash.clone();
    if !tokio::task::spawn_blocking(move || verify(&i.current_password, &test_hash))
        .await
        .map_err(internal)?
    {
        return Err(invalid("Mot de passe actuel incorrect."));
    }
    let hash = hash_async(i.new_password).await?;
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    let changed=tx.execute("UPDATE auth_accounts SET password_hash=?1 WHERE id=?2 AND password_hash=?3 AND enabled=1",params![hash,p.id,old_hash]).map_err(internal)?;
    if changed != 1 {
        return Err(unauthorized());
    }
    tx.execute("DELETE FROM auth_sessions WHERE account_id=?1", [&p.id])
        .map_err(internal)?;
    let token = create_session(&tx, &p.id)?;
    log(
        &tx,
        Some(&p.id),
        "Mot de passe changé, sessions précédentes révoquées",
    )?;
    tx.commit().map_err(internal)?;
    Ok(signed_in(p, token))
}
pub async fn catalog(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
) -> Result<Json<serde_json::Value>> {
    must_admin(&p)?;
    let db = app.db.lock().map_err(internal)?;
    let d = read(&db)?;
    Ok(Json(
        serde_json::json!({"patients":d.patients,"nurses":d.nurses,"pharmacies":d.pharmacies}),
    ))
}
pub async fn accounts(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
) -> Result<Json<Vec<Principal>>> {
    must_admin(&p)?;
    let db = app.db.lock().map_err(internal)?;
    let ids = db
        .prepare("SELECT id FROM auth_accounts ORDER BY created_at")
        .map_err(internal)?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(internal)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(internal)?;
    Ok(Json(
        ids.iter()
            .map(|id| principal(&db, id))
            .collect::<Result<Vec<_>>>()?,
    ))
}
pub async fn invite(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(i): Json<Invitation>,
) -> Result<Json<serde_json::Value>> {
    must_admin(&p)?;
    let uname = username(&i.username)?;
    required(&i.name, "Nom du titulaire", 120)?;
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    binding(&read(&tx)?, &i.role, &i.subject_id, &i.patient_ids)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM auth_accounts WHERE username=?1)",
            [&uname],
            |r| r.get(0),
        )
        .map_err(internal)?;
    if exists {
        return Err(invalid("Cet identifiant est déjà utilisé."));
    }
    let aid = id();
    let code = secret();
    let expires = Utc::now().timestamp() + 86400;
    tx.execute("INSERT INTO auth_accounts(id,username,name,role,subject_id,patient_ids,activation_hash,activation_expires,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![aid,uname,i.name.trim(),i.role,i.subject_id,serde_json::to_string(&i.patient_ids).map_err(internal)?,digest(&code),expires,Utc::now().to_rfc3339()]).map_err(internal)?;
    log(&tx, Some(&p.id), &format!("Invitation du compte {aid}"))?;
    tx.commit().map_err(internal)?;
    Ok(Json(
        serde_json::json!({"id":aid,"activation_code":code,"expires_at":expires,"activation_path":"/activate"}),
    ))
}
pub async fn update_access(
    State(app): State<App>,
    Path(aid): Path<String>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(i): Json<AccessUpdate>,
) -> Result<Json<serde_json::Value>> {
    must_admin(&p)?;
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    let existing = principal(&tx, &aid)?;
    binding(&read(&tx)?, &existing.role, &i.subject_id, &i.patient_ids)?;
    if aid == p.id && !i.enabled {
        return Err(invalid(
            "Vous ne pouvez pas désactiver votre propre compte.",
        ));
    }
    if existing.role == "admin" && existing.enabled && existing.activated && !i.enabled {
        let n:i64=tx.query_row("SELECT COUNT(*) FROM auth_accounts WHERE role='admin' AND enabled=1 AND password_hash IS NOT NULL",[],|r|r.get(0)).map_err(internal)?;
        if n <= 1 {
            return Err(invalid(
                "Le dernier administrateur actif doit être conservé.",
            ));
        }
    }
    tx.execute(
        "UPDATE auth_accounts SET subject_id=?1,patient_ids=?2,enabled=?3 WHERE id=?4",
        params![
            i.subject_id,
            serde_json::to_string(&i.patient_ids).map_err(internal)?,
            i.enabled,
            aid
        ],
    )
    .map_err(internal)?;
    tx.execute("DELETE FROM auth_sessions WHERE account_id=?1", [&aid])
        .map_err(internal)?;
    log(
        &tx,
        Some(&p.id),
        &format!("Droits du compte {aid} mis à jour ; sessions révoquées"),
    )?;
    tx.commit().map_err(internal)?;
    Ok(Json(serde_json::json!({"ok":true})))
}
pub async fn reissue(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(i): Json<Reissue>,
) -> Result<Json<serde_json::Value>> {
    must_admin(&p)?;
    let db = app.db.lock().map_err(internal)?;
    let a = principal(&db, &i.account_id)?;
    if a.activated || !a.enabled {
        return Err(invalid(
            "Seul un compte non activé et actif peut recevoir un nouveau code.",
        ));
    }
    let code = secret();
    let expires = Utc::now().timestamp() + 86400;
    db.execute(
        "UPDATE auth_accounts SET activation_hash=?1,activation_expires=?2 WHERE id=?3",
        params![digest(&code), expires, a.id],
    )
    .map_err(internal)?;
    log(
        &db,
        Some(&p.id),
        &format!("Code d’activation du compte {} remplacé", a.id),
    )?;
    Ok(Json(
        serde_json::json!({"activation_code":code,"expires_at":expires,"activation_path":"/activate"}),
    ))
}
pub async fn audit_log(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
) -> Result<Json<Vec<serde_json::Value>>> {
    must_admin(&p)?;
    let db = app.db.lock().map_err(internal)?;
    let rows=db.prepare("SELECT at,account_id,action FROM auth_audit ORDER BY rowid DESC LIMIT 200").map_err(internal)?.query_map([],|r|Ok(serde_json::json!({"at":r.get::<_,String>(0)?,"account_id":r.get::<_,Option<String>>(1)?,"action":r.get::<_,String>(2)?}))).map_err(internal)?.collect::<std::result::Result<Vec<_>,_>>().map_err(internal)?;
    Ok(Json(rows))
}

pub fn scope(p: &Principal, mut d: Data) -> Data {
    let ids: HashSet<String> = p.patient_ids.iter().cloned().collect();
    d.patients.retain(|x| ids.contains(&x.id));
    d.programs.retain(|x| ids.contains(&x.patient_id));
    d.appointments.retain(|x| ids.contains(&x.patient_id));
    d.prescriptions.retain(|x| ids.contains(&x.patient_id));
    d.renewals.retain(|x| ids.contains(&x.patient_id));
    d.visits.retain(|x| ids.contains(&x.patient_id));
    d.nursing_reports.retain(|x| ids.contains(&x.patient_id));
    d.rights_requests.retain(|x| ids.contains(&x.patient_id));
    d.protections.retain(|x| ids.contains(&x.patient_id));
    let mut entities = ids;
    for x in &d.programs {
        entities.insert(x.id.clone());
    }
    for x in &d.appointments {
        entities.insert(x.id.clone());
    }
    for x in &d.prescriptions {
        entities.insert(x.id.clone());
    }
    for x in &d.renewals {
        entities.insert(x.id.clone());
    }
    for x in &d.visits {
        entities.insert(x.id.clone());
    }
    for x in &d.nursing_reports {
        entities.insert(x.id.clone());
    }
    for x in &d.rights_requests {
        entities.insert(x.id.clone());
    }
    for x in &d.protections {
        entities.insert(x.id.clone());
    }
    d.history.retain(|x| entities.contains(&x.entity));
    d
}
fn own(p: &Principal, subject: &str) -> bool {
    p.subject_id.as_deref() == Some(subject)
}
fn authorize(
    p: &Principal,
    method: &Method,
    path: &str,
    json: &serde_json::Value,
    d: &Data,
) -> Result<()> {
    let get = *method == Method::GET;
    let post = *method == Method::POST;
    let put = *method == Method::PUT;
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let staff = p.role == "doctor" || p.role == "team";
    let doc = p.role == "doctor";
    let allowed = if path.starts_with("/api/auth/admin/") {
        p.role == "admin"
    } else if path == "/api/auth/me" {
        get
    } else if path == "/api/auth/logout" || path == "/api/auth/password" {
        post
    } else if p.role == "admin" {
        post && [
            "/api/patients",
            "/api/nurses",
            "/api/pharmacies",
            "/api/demo",
            "/api/nursing-demo",
            "/api/medication-demo",
        ]
        .contains(&path)
    } else if path == "/api/state" {
        get && staff
    } else if [
        "/api/patient-options",
        "/api/nurse-options",
        "/api/pharmacy-options",
    ]
    .contains(&path)
    {
        get && match path {
            "/api/patient-options" => p.role == "patient",
            "/api/nurse-options" => p.role == "nurse",
            _ => p.role == "pharmacy",
        }
    } else {
        match parts.as_slice() {
            ["api", "rights"] => {
                post && p.role == "patient"
                    && json
                        .get("program_id")
                        .and_then(|x| x.as_str())
                        .map(|id| {
                            d.programs
                                .iter()
                                .any(|x| x.id == id && own(p, &x.patient_id))
                        })
                        .unwrap_or(false)
            }
            ["api", "rights", id, "updates"] => {
                post && staff
                    && d.rights_requests
                        .iter()
                        .any(|r| r.id == *id && p.has_patient(&r.patient_id))
            }
            ["api", "patient", pid] => get && p.role == "patient" && own(p, pid),
            ["api", "nurse", nid] => get && p.role == "nurse" && own(p, nid),
            ["api", "pharmacy", fid] => get && p.role == "pharmacy" && own(p, fid),
            ["api", "nurse", nid, "visits", vid, "report"] => {
                post && p.role == "nurse"
                    && own(p, nid)
                    && d.visits.iter().any(|v| v.id == *vid && v.nurse_id == *nid)
            }
            ["api", "pharmacy", fid, "renewals", rid] => {
                put && p.role == "pharmacy"
                    && own(p, fid)
                    && d.renewals
                        .iter()
                        .any(|r| r.id == *rid && r.pharmacy_id == *fid)
            }
            ["api", "patients", pid, kind] => {
                put && staff
                    && ["nurse", "pharmacy", "protection"].contains(kind)
                    && p.has_patient(pid)
            }
            ["api", "programs"] => {
                post && doc
                    && json
                        .get("patient_id")
                        .and_then(|v| v.as_str())
                        .map(|id| p.has_patient(id))
                        .unwrap_or(false)
            }
            ["api", "programs", id] => {
                put && doc
                    && d.programs
                        .iter()
                        .any(|x| x.id == *id && p.has_patient(&x.patient_id))
            }
            ["api", "appointments"] | ["api", "visits"] => {
                post && staff
                    && json
                        .get("patient_id")
                        .and_then(|v| v.as_str())
                        .map(|id| p.has_patient(id))
                        .unwrap_or(false)
            }
            ["api", "appointments", id] => {
                put && staff
                    && d.appointments
                        .iter()
                        .any(|x| x.id == *id && p.has_patient(&x.patient_id))
            }
            ["api", "prescriptions"] => {
                post && doc
                    && json
                        .get("patient_id")
                        .and_then(|v| v.as_str())
                        .map(|id| p.has_patient(id))
                        .unwrap_or(false)
            }
            ["api", "prescriptions", id, "automation"] => {
                put && doc
                    && d.prescriptions
                        .iter()
                        .any(|x| x.id == *id && p.has_patient(&x.patient_id))
            }
            ["api", "prescriptions", id, "request"] => {
                post && d.prescriptions.iter().any(|x| {
                    x.id == *id
                        && ((staff && p.has_patient(&x.patient_id))
                            || (p.role == "patient" && own(p, &x.patient_id)))
                })
            }
            ["api", "renewals", id, "decision"] => {
                post && doc
                    && d.renewals
                        .iter()
                        .any(|x| x.id == *id && p.has_patient(&x.patient_id))
            }
            ["api", "nursing-reports", id, "review"] => {
                post && staff
                    && d.nursing_reports
                        .iter()
                        .any(|x| x.id == *id && p.has_patient(&x.patient_id))
            }
            ["api", "nurses"] | ["api", "pharmacies"] => post && staff,
            _ => false,
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(forbidden())
    }
}
fn returns_data(path: &str, method: &Method) -> bool {
    path == "/api/state"
        || ((*method == Method::POST || *method == Method::PUT)
            && !path.starts_with("/api/auth/")
            && !path.starts_with("/api/nurse/")
            && !path.starts_with("/api/pharmacy/"))
}
pub async fn protect(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let public = [
        "/login",
        "/setup",
        "/activate",
        "/auth.js",
        "/account.js",
        "/auth.css",
        "/style.css",
        "/patient.css",
        "/app.js",
        "/patient.js",
        "/nurse.js",
        "/nursing.js",
        "/pharmacy.js",
        "/medication.js",
        "/admin.js",
        "/rights.js",
    ];
    if public.contains(&path.as_str())
        || [
            "/api/auth/status",
            "/api/auth/setup",
            "/api/auth/login",
            "/api/auth/activate",
        ]
        .contains(&path.as_str())
    {
        return next.run(req).await;
    }
    let principal_result = (|| {
        let token = cookie_token(&req).ok_or_else(unauthorized)?;
        let db = app.db.lock().map_err(internal)?;
        session(&db, &token)
    })();
    let p = match principal_result {
        Ok(p) => p,
        Err(e) => {
            return if path.starts_with("/api/") {
                e.into_response()
            } else {
                Redirect::to("/login").into_response()
            }
        }
    };
    if !path.starts_with("/api/") {
        if ["/", "/admin", "/patient", "/pharmacie", "/domicile"].contains(&path.as_str())
            && path != p.home()
        {
            return Redirect::to(p.home()).into_response();
        }
        req.extensions_mut().insert(p);
        return next.run(req).await;
    }
    let mut json = serde_json::Value::Null;
    if (method == Method::POST || method == Method::PUT) && !path.starts_with("/api/auth/") {
        let body = std::mem::replace(req.body_mut(), Body::empty());
        let bytes = match to_bytes(body, 2 * 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => return invalid("Requête trop volumineuse.").into_response(),
        };
        if !bytes.is_empty() {
            json = match serde_json::from_slice(&bytes) {
                Ok(j) => j,
                Err(_) => return invalid("Contenu JSON invalide.").into_response(),
            };
        }
    }
    let authorized = (|| {
        let db = app.db.lock().map_err(internal)?;
        authorize(&p, &method, &path, &json, &read(&db)?)
    })();
    if let Err(e) = authorized {
        if let Ok(db) = app.db.lock() {
            let _ = log(
                &db,
                Some(&p.id),
                &format!("Accès refusé : {} {}", method, path),
            );
        }
        return e.into_response();
    }
    if !json.is_null() {
        if let Some(obj) = json.as_object_mut() {
            for key in ["actor", "author"] {
                obj.insert(key.into(), serde_json::Value::String(p.name.clone()));
            }
        }
        *req.body_mut() = Body::from(serde_json::to_vec(&json).unwrap_or_default());
        req.headers_mut().remove(header::CONTENT_LENGTH);
    }
    req.extensions_mut().insert(p.clone());
    let mut response = next.run(req).await;
    if response.status().is_success() && returns_data(&path, &method) {
        let (_parts, body) = response.into_parts();
        let data = match to_bytes(body, 32 * 1024 * 1024).await {
            Ok(bytes) => serde_json::from_slice::<Data>(&bytes).map_err(internal),
            Err(e) => Err(internal(e)),
        };
        response = match data {
            Ok(d) => {
                if p.role == "doctor" || p.role == "team" {
                    Json(scope(&p, d)).into_response()
                } else {
                    Json(serde_json::json!({"ok":true})).into_response()
                }
            }
            Err(e) => e.into_response(),
        };
    }
    if response.status().is_success() && method != Method::GET && !path.starts_with("/api/auth/") {
        if let Ok(db) = app.db.lock() {
            if log(&db, Some(&p.id), &format!("{} {}", method, path)).is_err() {
                return internal("audit").into_response();
            }
        } else {
            return internal("audit lock").into_response();
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use tower::ServiceExt;
    fn test_hash() -> String {
        static HASH: OnceLock<String> = OnceLock::new();
        HASH.get_or_init(|| hash_password("Fictitious-test-password-2026").unwrap())
            .clone()
    }
    pub(super) async fn fixture() -> (App, Data) {
        let app = open(std::path::Path::new(":memory:")).unwrap();
        let _ = seed(State(app.clone())).await.unwrap();
        let _ = medication::demo(State(app.clone())).await.unwrap();
        let _ = nursing::demo(State(app.clone())).await.unwrap();
        let d = read(&app.db.lock().unwrap()).unwrap();
        (app, d)
    }
    pub(super) fn account(
        app: &App,
        role: &str,
        subject: Option<String>,
        ids: Vec<String>,
    ) -> (Principal, String) {
        let aid = id();
        let db = app.db.lock().unwrap();
        db.execute("INSERT INTO auth_accounts(id,username,name,role,subject_id,patient_ids,password_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![aid,format!("test-{}",id()),format!("Compte {role}"),role,subject,serde_json::to_string(&ids).unwrap(),test_hash(),Utc::now().to_rfc3339()]).unwrap();
        let p = principal(&db, &aid).unwrap();
        let token = create_session(&db, &aid).unwrap();
        (p, format!("{AUTH_COOKIE}={token}"))
    }
    pub(super) async fn http(
        app: &App,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value, axum::http::HeaderMap) {
        let mut r = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:8080");
        if let Some(c) = cookie {
            r = r.header(header::COOKIE, c);
        }
        let raw = body
            .map(|b| serde_json::to_vec(&b).unwrap())
            .unwrap_or_default();
        let r = r
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(raw))
            .unwrap();
        let result = router(app.clone()).oneshot(r).await.unwrap();
        let status = result.status();
        let headers = result.headers().clone();
        let bytes = to_bytes(result.into_body(), 32 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
            headers,
        )
    }
    #[tokio::test]
    async fn unauthenticated_clinical_routes_are_closed() {
        let (app, d) = fixture().await;
        for path in [
            "/api/state".to_string(),
            format!("/api/patient/{}", d.patients[0].id),
            "/api/patient-options".into(),
            "/api/nurse-options".into(),
            "/api/pharmacy-options".into(),
            "/api/auth/admin/catalog".into(),
        ] {
            assert_eq!(
                http(&app, "GET", &path, None, None).await.0,
                StatusCode::UNAUTHORIZED,
                "{}",
                path
            );
        }
        let (status, _, headers) = http(&app, "GET", "/", None, None).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(headers[header::LOCATION], "/login");
    }
    #[tokio::test]
    async fn patient_cannot_read_or_request_for_another_patient() {
        let (app, d) = fixture().await;
        let (_, cookie) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let (_, options, _) = http(&app, "GET", "/api/patient-options", Some(&cookie), None).await;
        assert_eq!(options.as_array().unwrap().len(), 1);
        assert_eq!(options[0]["id"], d.patients[0].id);
        assert_eq!(
            http(&app, "GET", "/api/state", Some(&cookie), None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(
                &app,
                "GET",
                &format!("/api/patient/{}", d.patients[1].id),
                Some(&cookie),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(
                &app,
                "POST",
                &format!("/api/prescriptions/{}/request", d.prescriptions[1].id),
                Some(&cookie),
                Some(serde_json::json!({}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        mutate(&app, |d| {
            d.renewals.clear();
            Ok(())
        })
        .unwrap();
        let (status, body, _) = http(
            &app,
            "POST",
            &format!("/api/prescriptions/{}/request", d.prescriptions[0].id),
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok":true}));
    }
    #[tokio::test]
    async fn staff_state_and_mutation_responses_are_scoped() {
        let (app, d) = fixture().await;
        let (_, cookie) = account(&app, "team", None, vec![d.patients[0].id.clone()]);
        let (status, body, _) = http(&app, "GET", "/api/state", Some(&cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["patients"].as_array().unwrap().len(), 1);
        for key in [
            "programs",
            "appointments",
            "prescriptions",
            "renewals",
            "visits",
        ] {
            assert!(
                body[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|x| x["patient_id"] == d.patients[0].id),
                "{key}"
            );
        }
        let(status,body,_)=http(&app,"POST","/api/appointments",Some(&cookie),Some(serde_json::json!({"patient_id":d.patients[0].id,"date":"2030-01-01","time":"09:00","kind":"Fictif","professional":"Dr Fictif","place":"CMP"}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["patients"].as_array().unwrap().len(), 1);
        assert_eq!(http(&app,"POST","/api/appointments",Some(&cookie),Some(serde_json::json!({"patient_id":d.patients[1].id,"date":"2030-01-01","time":"09:00","kind":"Fictif","professional":"Dr","place":"CMP"}))).await.0,StatusCode::FORBIDDEN);
    }
    #[tokio::test]
    async fn only_assigned_doctor_can_decide_and_actor_is_bound() {
        let (app, d) = fixture().await;
        let (_, team) = account(&app, "team", None, vec![d.patients[0].id.clone()]);
        let (doc, doctor) = account(&app, "doctor", None, vec![d.patients[0].id.clone()]);
        let body = serde_json::json!({"approved":false,"actor":"Usurped name","confirmed":true});
        let path = format!("/api/renewals/{}/decision", d.renewals[0].id);
        assert_eq!(
            http(&app, "POST", &path, Some(&team), Some(body.clone()))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(
                &app,
                "POST",
                &format!("/api/renewals/{}/decision", d.renewals[1].id),
                Some(&doctor),
                Some(body.clone())
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(&app, "POST", &path, Some(&doctor), Some(body)).await.0,
            StatusCode::OK
        );
        let after = read(&app.db.lock().unwrap()).unwrap();
        assert_eq!(
            after.renewals[0].decided_by.as_deref(),
            Some(doc.name.as_str())
        );
    }
    #[tokio::test]
    async fn nurse_is_limited_to_own_profile_and_actor_is_bound() {
        let (app, d) = fixture().await;
        let (p, cookie) = account(&app, "nurse", Some(d.nurses[0].id.clone()), vec![]);
        assert_eq!(
            http(&app, "GET", "/api/nurse/not-mine", Some(&cookie), None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(&app, "GET", "/api/pharmacy-options", Some(&cookie), None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let body = serde_json::json!({"status":"Réalisée","actor":"Spoof","availability":"Non vérifiée","intake":"Non évaluée","note":"Observation fictive","needs_review":true});
        let (status, view, _) = http(
            &app,
            "POST",
            &format!(
                "/api/nurse/{}/visits/{}/report",
                d.nurses[0].id, d.visits[0].id
            ),
            Some(&cookie),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["reports"][0]["actor"], p.name);
        assert_eq!(view["patients"].as_array().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn pharmacy_cannot_access_other_officines_or_validate() {
        let (app, d) = fixture().await;
        let (_, cookie) = account(&app, "pharmacy", Some(d.pharmacies[0].id.clone()), vec![]);
        assert_eq!(
            http(&app, "GET", "/api/pharmacy/another", Some(&cookie), None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(&app, "GET", "/api/state", Some(&cookie), None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(
                &app,
                "POST",
                &format!("/api/renewals/{}/decision", d.renewals[0].id),
                Some(&cookie),
                Some(serde_json::json!({"approved":false,"actor":"Ph","confirmed":true}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn administrators_manage_metadata_without_clinical_access() {
        let (app, _) = fixture().await;
        let (_, cookie) = account(&app, "admin", None, vec![]);
        assert_eq!(
            http(&app, "GET", "/api/state", Some(&cookie), None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(&app, "GET", "/api/auth/admin/catalog", Some(&cookie), None)
                .await
                .0,
            StatusCode::OK
        );
        let (status, body, _) = http(
            &app,
            "POST",
            "/api/patients",
            Some(&cookie),
            Some(
                serde_json::json!({"alias":"Nouveau fictif","reference":"Dr Fictif","team":"CMP"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok":true}));
    }
    #[tokio::test]
    async fn disabled_expired_and_logged_out_sessions_are_rejected() {
        let (app, d) = fixture().await;
        let (p, cookie) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let (_, logout, _) = http(&app, "POST", "/api/auth/logout", Some(&cookie), None).await;
        assert_eq!(logout["ok"], true);
        assert_eq!(
            http(&app, "GET", "/api/auth/me", Some(&cookie), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let token = create_session(&app.db.lock().unwrap(), &p.id).unwrap();
        let expired = format!("{AUTH_COOKIE}={token}");
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE auth_sessions SET expires_at=0", [])
            .unwrap();
        assert_eq!(
            http(&app, "GET", "/api/auth/me", Some(&expired), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let token = create_session(&app.db.lock().unwrap(), &p.id).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE auth_accounts SET enabled=0 WHERE id=?1", [p.id])
            .unwrap();
        assert_eq!(
            http(
                &app,
                "GET",
                "/api/auth/me",
                Some(&format!("{AUTH_COOKIE}={token}")),
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn setup_once_activation_once_no_password_or_tokens_in_account_list() {
        let app = open(std::path::Path::new(":memory:")).unwrap();
        let (status,body,headers)=http(&app,"POST","/api/auth/setup",None,Some(serde_json::json!({"username":"admin-test","name":"Admin Test","password":"Fictitious-test-password-2026"}))).await;
        assert_eq!(status, StatusCode::OK);
        let cookie = headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let header = headers[header::SET_COOKIE].to_str().unwrap();
        assert!(header.contains("HttpOnly") && header.contains("SameSite=Strict"));
        assert_eq!(body["home"], "/admin");
        assert_eq!(http(&app,"POST","/api/auth/setup",None,Some(serde_json::json!({"username":"another","name":"Other","password":"Another-fictitious-pass"}))).await.0,StatusCode::FORBIDDEN);
        let(status,invitation,_)=http(&app,"POST","/api/auth/admin/accounts",Some(&cookie),Some(serde_json::json!({"username":"admin-second","name":"Second Test","role":"admin","subject_id":null,"patient_ids":[]}))).await;
        assert_eq!(status, StatusCode::OK);
        let code = invitation["activation_code"].as_str().unwrap();
        let activation = serde_json::json!({"code":code,"password":"Second-fictitious-password"});
        assert_eq!(
            http(
                &app,
                "POST",
                "/api/auth/activate",
                None,
                Some(activation.clone())
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            http(&app, "POST", "/api/auth/activate", None, Some(activation))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let (_, list, _) = http(&app, "GET", "/api/auth/admin/accounts", Some(&cookie), None).await;
        let raw = list.to_string();
        assert!(
            !raw.contains("password_hash")
                && !raw.contains(code)
                && !raw.contains("Fictitious-test-password")
        );
        let db = app.db.lock().unwrap();
        let stored: String = db
            .query_row(
                "SELECT password_hash FROM auth_accounts WHERE username='admin-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert_ne!(stored, "Fictitious-test-password-2026");
    }
    #[tokio::test]
    async fn permission_updates_revoke_sessions_and_cannot_disable_self() {
        let (app, d) = fixture().await;
        let (admin, admin_cookie) = account(&app, "admin", None, vec![]);
        let (doc, cookie) = account(&app, "doctor", None, vec![d.patients[0].id.clone()]);
        let update =
            serde_json::json!({"subject_id":null,"patient_ids":[d.patients[1].id],"enabled":true});
        assert_eq!(
            http(
                &app,
                "PUT",
                &format!("/api/auth/admin/accounts/{}", doc.id),
                Some(&admin_cookie),
                Some(update)
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            http(&app, "GET", "/api/state", Some(&cookie), None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            http(
                &app,
                "PUT",
                &format!("/api/auth/admin/accounts/{}", admin.id),
                Some(&admin_cookie),
                Some(serde_json::json!({"subject_id":null,"patient_ids":[],"enabled":false}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn login_password_change_and_cookie_revocation() {
        let (app, d) = fixture().await;
        let (p, old_cookie) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let(status,_,headers)=http(&app,"POST","/api/auth/login",None,Some(serde_json::json!({"username":p.username,"password":"Fictitious-test-password-2026"}))).await;
        assert_eq!(status, StatusCode::OK);
        let cookie = headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let(status,_,headers)=http(&app,"POST","/api/auth/password",Some(cookie),Some(serde_json::json!({"current_password":"Fictitious-test-password-2026","new_password":"Updated-fictitious-password"}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            http(&app, "GET", "/api/auth/me", Some(&old_cookie), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            http(&app, "GET", "/api/auth/me", Some(cookie), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let new_cookie = headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        assert_eq!(
            http(&app, "GET", "/api/auth/me", Some(new_cookie), None)
                .await
                .0,
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn login_is_throttled_and_cross_origin_is_rejected() {
        let (app, d) = fixture().await;
        let (p, _) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        for _ in 0..5 {
            assert_eq!(
                http(
                    &app,
                    "POST",
                    "/api/auth/login",
                    None,
                    Some(serde_json::json!({"username":p.username,"password":"wrong"}))
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(http(&app,"POST","/api/auth/login",None,Some(serde_json::json!({"username":p.username,"password":"Fictitious-test-password-2026"}))).await.0,StatusCode::TOO_MANY_REQUESTS);
        let request = axum::http::Request::builder()
            .uri("/api/auth/status")
            .header(header::HOST, "127.0.0.1:8080")
            .header(header::ORIGIN, "https://example.org")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router(app).oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[cfg(test)]
mod rights_access_tests {
    use super::tests::{account, fixture, http};
    use super::*;
    #[tokio::test]
    async fn patient_can_request_assistance_without_medical_approval_and_other_roles_cannot_read_it(
    ) {
        let (app, d) = fixture().await;
        let (patient, cookie) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let payload = serde_json::json!({"program_id":d.programs[0].id,"lawyer":true,"independent_opinion":true,"message":"Exemple fictif : accompagnement avocat et avis indépendant.","share_with_team":true});
        let (status, body, _) = http(
            &app,
            "POST",
            "/api/rights",
            Some(&cookie),
            Some(payload.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok":true}));
        let after = read(&app.db.lock().unwrap()).unwrap();
        let request = &after.rights_requests[0];
        assert_eq!(request.submitted_by, patient.id);
        assert_eq!(request.program_version, 1);
        assert!(request.updates.is_empty());
        let (_, doctor) = account(&app, "doctor", None, vec![d.patients[1].id.clone()]);
        let (_, state, _) = http(&app, "GET", "/api/state", Some(&doctor), None).await;
        assert!(state["rights_requests"].as_array().unwrap().is_empty());
        let (_, own_doctor) = account(&app, "doctor", None, vec![d.patients[0].id.clone()]);
        let (_, state, _) = http(&app, "GET", "/api/state", Some(&own_doctor), None).await;
        assert_eq!(state["rights_requests"].as_array().unwrap().len(), 1);
        let (_, patient_view, _) = http(
            &app,
            "GET",
            &format!("/api/patient/{}", d.patients[0].id),
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(patient_view["rights_requests"].as_array().unwrap().len(), 1);
        let (_, nurse) = account(&app, "nurse", Some(d.nurses[0].id.clone()), vec![]);
        let (_, nv, _) = http(
            &app,
            "GET",
            &format!("/api/nurse/{}", d.nurses[0].id),
            Some(&nurse),
            None,
        )
        .await;
        assert!(!nv.to_string().contains("accompagnement avocat"));
        let (_, pharmacy) = account(&app, "pharmacy", Some(d.pharmacies[0].id.clone()), vec![]);
        assert_eq!(
            http(&app, "POST", "/api/rights", Some(&pharmacy), Some(payload))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn assistance_updates_are_scoped_and_do_not_change_program() {
        let (app, d) = fixture().await;
        let (_, cookie) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let mut payload = serde_json::json!({"program_id":d.programs[0].id,"lawyer":true,"independent_opinion":true,"message":"Fictif","share_with_team":false});
        assert_eq!(
            http(
                &app,
                "POST",
                "/api/rights",
                Some(&cookie),
                Some(payload.clone())
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        payload["share_with_team"] = true.into();
        payload["program_id"] = d.programs[1].id.clone().into();
        assert_eq!(
            http(
                &app,
                "POST",
                "/api/rights",
                Some(&cookie),
                Some(payload.clone())
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        payload["program_id"] = d.programs[0].id.clone().into();
        assert_eq!(
            http(&app, "POST", "/api/rights", Some(&cookie), Some(payload))
                .await
                .0,
            StatusCode::OK
        );
        let rid = read(&app.db.lock().unwrap()).unwrap().rights_requests[0]
            .id
            .clone();
        let path = format!("/api/rights/{rid}/updates");
        let update = serde_json::json!({"status":"Accompagnement en cours","note":"Exemple fictif : demande prise en compte."});
        assert_eq!(
            http(&app, "POST", &path, Some(&cookie), Some(update.clone()))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let (p, staff) = account(&app, "team", None, vec![d.patients[0].id.clone()]);
        let (_, other) = account(&app, "team", None, vec![d.patients[1].id.clone()]);
        assert_eq!(
            http(&app, "POST", &path, Some(&other), Some(update.clone()))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http(&app, "POST", &path, Some(&staff), Some(update))
                .await
                .0,
            StatusCode::OK
        );
        let after = read(&app.db.lock().unwrap()).unwrap();
        assert_eq!(after.rights_requests[0].updates[0].actor_id, p.id);
        assert_eq!(
            serde_json::to_value(&after.programs).unwrap(),
            serde_json::to_value(&d.programs).unwrap()
        );
    }
}

#[cfg(test)]
mod cookie_tests {
    use super::*;
    #[test]
    fn session_cookie_respects_https_configuration() {
        let p = Principal {
            id: "test".into(),
            username: "test".into(),
            name: "Test".into(),
            role: "patient".into(),
            subject_id: Some("test".into()),
            patient_ids: vec![],
            enabled: true,
            activated: true,
        };
        let response = signed_in(p, "synthetic-cookie-for-format-check".into());
        let value = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(value.contains("HttpOnly") && value.contains("SameSite=Strict"));
        assert_eq!(
            value.contains("; Secure"),
            !secure_cookie_suffix().is_empty()
        );
    }
}

#[cfg(test)]
mod protection_access_tests {
    use super::tests::{account, fixture, http};
    use super::*;
    fn payload() -> serde_json::Value {
        serde_json::json!({"kind":"Curatelle renforcée","status":"En cours","starts_on":"2026-01-01","ends_on":"2030-01-01","judgment_reference":"JUGEMENT-FICTIF","scope":"Assistance à préciser selon la décision, sans attribution automatique d’accès.","personal_assistance":"À vérifier dans la décision","curator_name":"Curatrice fictive confidentielle","organization":"Service fictif","phone":"","email":""})
    }
    #[tokio::test]
    async fn measure_is_scoped_and_does_not_grant_a_curator_or_care_partners_access() {
        let (app, d) = fixture().await;
        let (doctor, dc) = account(&app, "doctor", None, vec![d.patients[0].id.clone()]);
        let (_, other) = account(&app, "team", None, vec![d.patients[1].id.clone()]);
        let (_, patient) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let (_, admin) = account(&app, "admin", None, vec![]);
        let route = format!("/api/patients/{}/protection", d.patients[0].id);
        for cookie in [&other, &patient, &admin] {
            assert_eq!(
                http(&app, "PUT", &route, Some(cookie), Some(payload()))
                    .await
                    .0,
                StatusCode::FORBIDDEN
            );
        }
        let n: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM auth_accounts", [], |r| r.get(0))
            .unwrap();
        let (status, state, _) = http(&app, "PUT", &route, Some(&dc), Some(payload())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            state["protections"][0]["versions"][0]["actor_id"],
            doctor.id
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM auth_accounts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            n
        );
        let (_, own, _) = http(
            &app,
            "GET",
            &format!("/api/patient/{}", d.patients[0].id),
            Some(&patient),
            None,
        )
        .await;
        assert_eq!(
            own["protection"]["versions"][0]["kind"],
            "Curatelle renforcée"
        );
        let (_, other_state, _) = http(&app, "GET", "/api/state", Some(&other), None).await;
        assert!(other_state["protections"].as_array().unwrap().is_empty());
        let (_, nurse) = account(&app, "nurse", Some(d.nurses[0].id.clone()), vec![]);
        let (_, nv, _) = http(
            &app,
            "GET",
            &format!("/api/nurse/{}", d.nurses[0].id),
            Some(&nurse),
            None,
        )
        .await;
        assert!(!nv.to_string().contains("Curatrice fictive confidentielle"));
        let (_, pharmacy) = account(&app, "pharmacy", Some(d.pharmacies[0].id.clone()), vec![]);
        let (_, fv, _) = http(
            &app,
            "GET",
            &format!("/api/pharmacy/{}", d.pharmacies[0].id),
            Some(&pharmacy),
            None,
        )
        .await;
        assert!(!fv.to_string().contains("Curatrice fictive confidentielle"));
    }
    #[tokio::test]
    async fn patient_rights_remain_available_and_curator_assistance_is_optional_and_versioned() {
        let (app, d) = fixture().await;
        let (_, dc) = account(&app, "doctor", None, vec![d.patients[0].id.clone()]);
        let (_, pc) = account(&app, "patient", Some(d.patients[0].id.clone()), vec![]);
        let path = format!("/api/patients/{}/protection", d.patients[0].id);
        assert_eq!(
            http(&app, "PUT", &path, Some(&dc), Some(payload())).await.0,
            StatusCode::OK
        );
        let mut body = serde_json::json!({"program_id":d.programs[0].id,"lawyer":true,"independent_opinion":true,"message":"Demande fictive d’accompagnement.","share_with_team":true});
        assert_eq!(
            http(&app, "POST", "/api/rights", Some(&pc), Some(body.clone()))
                .await
                .0,
            StatusCode::OK
        );
        body["include_curator"] = true.into();
        assert_eq!(
            http(&app, "POST", "/api/rights", Some(&pc), Some(body))
                .await
                .0,
            StatusCode::OK
        );
        let mut changed = payload();
        changed["kind"] = "Curatelle aménagée".into();
        assert_eq!(
            http(&app, "PUT", &path, Some(&dc), Some(changed)).await.0,
            StatusCode::OK
        );
        let after = read(&app.db.lock().unwrap()).unwrap();
        assert_eq!(after.protections[0].versions.len(), 2);
        assert!(!after.rights_requests[0].curator_assistance_requested);
        assert!(after.rights_requests[1].curator_assistance_requested);
        assert_eq!(after.rights_requests[1].protection_version, Some(1));
        assert_eq!(
            serde_json::to_value(&after.programs).unwrap(),
            serde_json::to_value(&d.programs).unwrap()
        );
    }
}

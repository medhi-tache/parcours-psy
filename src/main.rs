mod protection;
use protection::Protection;
mod rights;
use rights::RightsRequest;
mod auth;
mod nursing;
use nursing::{Nurse, Report, Visit};
mod medication;
use axum::{
    extract::{Path, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{NaiveDate, NaiveTime, Utc};
use medication::{Pharmacy, Prescription, Renewal};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, Default)]
struct Data {
    patients: Vec<Patient>,
    programs: Vec<Program>,
    appointments: Vec<Appointment>,
    history: Vec<Event>,
    #[serde(default)]
    pharmacies: Vec<Pharmacy>,
    #[serde(default)]
    prescriptions: Vec<Prescription>,
    #[serde(default)]
    renewals: Vec<Renewal>,
    #[serde(default)]
    nurses: Vec<Nurse>,
    #[serde(default)]
    visits: Vec<Visit>,
    #[serde(default)]
    nursing_reports: Vec<Report>,
    #[serde(default)]
    rights_requests: Vec<RightsRequest>,
    #[serde(default)]
    protections: Vec<Protection>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Patient {
    id: String,
    alias: String,
    reference: String,
    team: String,
    #[serde(default)]
    pharmacy_id: Option<String>,
    #[serde(default)]
    nurse_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Version {
    number: usize,
    created_at: String,
    author: String,
    framework: String,
    modalities: String,
    start: String,
    review: String,
    status: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Program {
    id: String,
    patient_id: String,
    versions: Vec<Version>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Appointment {
    id: String,
    patient_id: String,
    date: String,
    time: String,
    kind: String,
    professional: String,
    place: String,
    status: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Event {
    id: String,
    at: String,
    action: String,
    entity: String,
}
#[derive(Deserialize)]
struct PatientInput {
    alias: String,
    reference: String,
    team: String,
}
#[derive(Deserialize)]
struct ProgramInput {
    patient_id: String,
    author: String,
    framework: String,
    modalities: String,
    start: String,
    review: String,
    status: String,
}
#[derive(Deserialize)]
struct AppointmentInput {
    patient_id: String,
    date: String,
    time: String,
    kind: String,
    professional: String,
    place: String,
}
#[derive(Deserialize)]
struct StatusInput {
    status: String,
}
#[derive(Clone)]
struct App {
    db: Arc<Mutex<Connection>>,
    allowed_origins: Arc<Vec<String>>,
    allowed_hosts: Arc<Vec<String>>,
}
#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error": self.1}))).into_response()
    }
}
type Result<T> = std::result::Result<T, ApiError>;
fn invalid(s: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, s.into())
}
fn internal(_: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Échec de l’enregistrement. Réessayez.".into(),
    )
}
fn required(s: &str, name: &str, max: usize) -> Result<()> {
    if s.trim().is_empty() || s.chars().count() > max {
        return Err(invalid(&format!(
            "{name} est requis ({max} caractères maximum)."
        )));
    }
    Ok(())
}
fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| invalid("Date invalide."))
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn audit(data: &mut Data, action: &str, entity: String) {
    data.history.push(Event {
        id: id(),
        at: Utc::now().to_rfc3339(),
        action: action.into(),
        entity,
    });
}
fn read(db: &Connection) -> Result<Data> {
    let json: String = db
        .query_row("SELECT content FROM state WHERE id=1", [], |r| r.get(0))
        .map_err(internal)?;
    serde_json::from_str(&json).map_err(internal)
}
fn mutate<F>(app: &App, f: F) -> Result<Data>
where
    F: FnOnce(&mut Data) -> Result<()>,
{
    let mut db = app.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    let mut data = read(&tx)?;
    f(&mut data)?;
    let json = serde_json::to_string(&data).map_err(internal)?;
    tx.execute("UPDATE state SET content=?1 WHERE id=1", params![json])
        .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(data)
}
fn open(path: &std::path::Path) -> std::result::Result<App, Box<dyn std::error::Error>> {
    let db = Connection::open(path)?;
    #[cfg(unix)]
    if path != std::path::Path::new(":memory:") {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1),content TEXT NOT NULL);")?;
    auth::init(&db)?;
    db.execute(
        "INSERT OR IGNORE INTO state(id,content) VALUES(1,?1)",
        [serde_json::to_string(&Data::default())?],
    )?;
    let allowed_origins = if let Ok(origin) = std::env::var("PARCOURS_FRONTEND_ORIGIN") {
        vec![origin.trim_end_matches('/').to_string()]
    } else {
        vec![
            "http://127.0.0.1:8080".into(),
            "http://localhost:8080".into(),
            "http://127.0.0.1:3000".into(),
            "http://localhost:3000".into(),
        ]
    };
    let allowed_hosts = if let Ok(host) = std::env::var("PARCOURS_API_HOST") {
        vec![host]
    } else {
        vec!["127.0.0.1:8080".into(), "localhost:8080".into()]
    };
    Ok(App {
        allowed_origins: Arc::new(allowed_origins),
        allowed_hosts: Arc::new(allowed_hosts),
        db: Arc::new(Mutex::new(db)),
    })
}
async fn get_state(State(app): State<App>) -> Result<Json<Data>> {
    let db = app.db.lock().map_err(internal)?;
    Ok(Json(read(&db)?))
}
fn add_patient(data: &mut Data, input: PatientInput) -> Result<()> {
    required(&input.alias, "Identifiant fictif", 80)?;
    required(&input.reference, "Référent", 100)?;
    required(&input.team, "Équipe", 100)?;
    if data
        .patients
        .iter()
        .any(|p| p.alias.eq_ignore_ascii_case(input.alias.trim()))
    {
        return Err(invalid("Cet identifiant existe déjà."));
    }
    let p = Patient {
        id: id(),
        alias: input.alias.trim().into(),
        reference: input.reference.trim().into(),
        team: input.team.trim().into(),
        pharmacy_id: None,
        nurse_id: None,
    };
    audit(data, "Patient ajouté", p.id.clone());
    data.patients.push(p);
    Ok(())
}
async fn create_patient(
    State(app): State<App>,
    Json(input): Json<PatientInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| add_patient(d, input))?))
}
fn version(input: &ProgramInput, number: usize) -> Result<Version> {
    required(&input.author, "Auteur", 100)?;
    required(&input.framework, "Cadre renseigné par l’équipe", 200)?;
    required(&input.modalities, "Modalités", 4000)?;
    if date(&input.review)? < date(&input.start)? {
        return Err(invalid("L’échéance doit suivre le début du programme."));
    }
    if !["Actif", "À réviser", "Terminé"].contains(&input.status.as_str()) {
        return Err(invalid("Statut invalide."));
    }
    Ok(Version {
        number,
        created_at: Utc::now().to_rfc3339(),
        author: input.author.trim().into(),
        framework: input.framework.trim().into(),
        modalities: input.modalities.trim().into(),
        start: input.start.clone(),
        review: input.review.clone(),
        status: input.status.clone(),
    })
}
fn patient_exists(d: &Data, id: &str) -> Result<()> {
    if d.patients.iter().any(|p| p.id == id) {
        Ok(())
    } else {
        Err(invalid("Patient inconnu."))
    }
}
fn add_program(d: &mut Data, input: ProgramInput) -> Result<()> {
    patient_exists(d, &input.patient_id)?;
    let v = version(&input, 1)?;
    let p = Program {
        id: id(),
        patient_id: input.patient_id,
        versions: vec![v],
    };
    audit(d, "Programme créé · version 1", p.id.clone());
    d.programs.push(p);
    Ok(())
}
async fn create_program(
    State(app): State<App>,
    Json(input): Json<ProgramInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| add_program(d, input))?))
}
fn revise(d: &mut Data, pid: String, input: ProgramInput) -> Result<()> {
    let p = d.programs.iter_mut().find(|p| p.id == pid).ok_or(ApiError(
        StatusCode::NOT_FOUND,
        "Programme introuvable.".into(),
    ))?;
    if p.patient_id != input.patient_id {
        return Err(invalid(
            "Le patient d’un programme ne peut pas être changé.",
        ));
    }
    let v = version(&input, p.versions.len() + 1)?;
    let n = v.number;
    p.versions.push(v);
    audit(d, &format!("Programme révisé · version {n}"), pid);
    Ok(())
}
async fn revise_program(
    State(app): State<App>,
    Path(pid): Path<String>,
    Json(input): Json<ProgramInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| revise(d, pid, input))?))
}
async fn create_appointment(
    State(app): State<App>,
    Json(input): Json<AppointmentInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        patient_exists(d, &input.patient_id)?;
        date(&input.date)?;
        NaiveTime::parse_from_str(&input.time, "%H:%M").map_err(|_| invalid("Heure invalide."))?;
        required(&input.kind, "Type", 100)?;
        required(&input.professional, "Professionnel", 100)?;
        required(&input.place, "Lieu", 200)?;
        let a = Appointment {
            id: id(),
            patient_id: input.patient_id,
            date: input.date,
            time: input.time,
            kind: input.kind,
            professional: input.professional,
            place: input.place,
            status: "Prévu".into(),
        };
        audit(d, "Rendez-vous planifié", a.id.clone());
        d.appointments.push(a);
        Ok(())
    })?))
}
async fn update_appointment(
    State(app): State<App>,
    Path(aid): Path<String>,
    Json(input): Json<StatusInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        if !["Prévu", "Réalisé", "Absent", "Annulé"].contains(&input.status.as_str()) {
            return Err(invalid("Statut invalide."));
        }
        let a = d
            .appointments
            .iter_mut()
            .find(|a| a.id == aid)
            .ok_or(ApiError(
                StatusCode::NOT_FOUND,
                "Rendez-vous introuvable.".into(),
            ))?;
        a.status = input.status.clone();
        audit(d, &format!("Rendez-vous : {}", input.status), aid);
        Ok(())
    })?))
}
async fn seed(State(app): State<App>) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        if !d.patients.is_empty() {
            return Err(invalid(
                "Les exemples se chargent uniquement dans une base vide.",
            ));
        }
        let today = chrono::Local::now().date_naive();
        for (i, (alias, r, team)) in [
            ("Patient Démo A", "Dr Martin", "CMP Nord"),
            ("Patient Démo B", "Dr Bernard", "CMP Centre"),
            ("Patient Démo C", "Dr Petit", "CMP Nord"),
            ("Patient Démo D", "Dr Robert", "CMP Est"),
            ("Patient Démo E", "Dr Durand", "CMP Centre"),
        ]
        .iter()
        .enumerate()
        {
            add_patient(
                d,
                PatientInput {
                    alias: (*alias).into(),
                    reference: (*r).into(),
                    team: (*team).into(),
                },
            )?;
            let p = d.patients.last().unwrap().id.clone();
            add_program(d,ProgramInput{patient_id:p.clone(),author:(*r).into(),framework:"Cadre de démonstration — à renseigner".into(),modalities:"Consultation au CMP : fréquence définie par l’équipe.\nEntretien infirmier : selon le programme validé.".into(),start:(today-chrono::Duration::days(20)).to_string(),review:(today+chrono::Duration::days([2,5,12,-1,20][i])).to_string(),status:if i==3 {"À réviser"} else {"Actif"}.into()})?;
            d.appointments.push(Appointment {
                id: id(),
                patient_id: p,
                date: (today + chrono::Duration::days(if i < 3 { 0 } else { 1 })).to_string(),
                time: format!("{:02}:00", 9 + i),
                kind: if i % 2 == 0 {
                    "Consultation psychiatrique"
                } else {
                    "Entretien infirmier"
                }
                .into(),
                professional: (*r).into(),
                place: (*team).into(),
                status: "Prévu".into(),
            });
        }
        audit(d, "Exemples fictifs chargés", "demo".into());
        Ok(())
    })?))
}
#[derive(Serialize)]
struct PatientOption {
    id: String,
    alias: String,
}
#[derive(Serialize)]
struct PatientProgram {
    id: String,
    version: Version,
}
#[derive(Serialize)]
struct PatientView {
    patient: Patient,
    programs: Vec<PatientProgram>,
    appointments: Vec<Appointment>,
    prescriptions: Vec<Prescription>,
    renewals: Vec<Renewal>,
    pharmacy: Option<Pharmacy>,
    nurse: Option<Nurse>,
    visits: Vec<Visit>,
    rights_requests: Vec<RightsRequest>,
    protection: Option<Protection>,
}
async fn patient_options(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<auth::Principal>,
) -> Result<Json<Vec<PatientOption>>> {
    let db = app.db.lock().map_err(internal)?;
    Ok(Json(
        read(&db)?
            .patients
            .into_iter()
            .filter(|x| p.subject_id.as_ref() == Some(&x.id))
            .map(|p| PatientOption {
                id: p.id,
                alias: p.alias,
            })
            .collect(),
    ))
}
async fn patient_view(
    State(app): State<App>,
    Path(pid): Path<String>,
) -> Result<Json<PatientView>> {
    let db = app.db.lock().map_err(internal)?;
    let data = read(&db)?;
    let p = data
        .patients
        .into_iter()
        .find(|p| p.id == pid)
        .ok_or(ApiError(
            StatusCode::NOT_FOUND,
            "Patient fictif introuvable.".into(),
        ))?;
    let programs = data
        .programs
        .into_iter()
        .filter(|p| p.patient_id == pid)
        .filter_map(|p| {
            p.versions
                .last()
                .cloned()
                .map(|version| PatientProgram { id: p.id, version })
        })
        .collect();
    let appointments = data
        .appointments
        .into_iter()
        .filter(|a| a.patient_id == pid)
        .collect();
    let prescriptions = data
        .prescriptions
        .into_iter()
        .filter(|x| x.patient_id == pid)
        .collect();
    let renewals = data
        .renewals
        .into_iter()
        .filter(|x| x.patient_id == pid)
        .collect();
    let pharmacy = data
        .pharmacies
        .into_iter()
        .find(|x| Some(&x.id) == p.pharmacy_id.as_ref());
    let nurse = data
        .nurses
        .into_iter()
        .find(|n| Some(&n.id) == p.nurse_id.as_ref());
    let visits = data
        .visits
        .into_iter()
        .filter(|v| v.patient_id == pid)
        .collect();
    let rights_requests = data
        .rights_requests
        .into_iter()
        .filter(|r| r.patient_id == pid)
        .collect();
    let protection = data.protections.into_iter().find(|x| x.patient_id == pid);
    Ok(Json(PatientView {
        protection,
        rights_requests,
        nurse,
        visits,
        prescriptions,
        renewals,
        pharmacy,
        patient: p,
        programs,
        appointments,
    }))
}
async fn asset(Path(name): Path<String>) -> Response {
    match name.as_str() {
        "rights.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/rights.js"),
        )
            .into_response(),
        "auth.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/auth.js"),
        )
            .into_response(),
        "account.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/account.js"),
        )
            .into_response(),
        "admin.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/admin.js"),
        )
            .into_response(),
        "auth.css" => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            include_str!("../static/auth.css"),
        )
            .into_response(),
        "nursing.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/nursing.js"),
        )
            .into_response(),
        "nurse.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/nurse.js"),
        )
            .into_response(),
        "medication.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/medication.js"),
        )
            .into_response(),
        "pharmacy.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/pharmacy.js"),
        )
            .into_response(),
        "patient.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/patient.js"),
        )
            .into_response(),
        "patient.css" => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            include_str!("../static/patient.css"),
        )
            .into_response(),
        "style.css" => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            include_str!("../static/style.css"),
        )
            .into_response(),
        "app.js" => (
            [(
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            )],
            include_str!("../static/app.js"),
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn guard(State(app): State<App>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let valid_host = app.allowed_hosts.iter().any(|h| h == host);
    let valid_origin = req
        .headers()
        .get(header::ORIGIN)
        .map(|v| {
            v.to_str()
                .map(|s| app.allowed_origins.iter().any(|origin| origin == s))
                .unwrap_or(false)
        })
        .unwrap_or(true);
    if !valid_host || !valid_origin {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(req).await;
    for (name,val) in [("x-content-type-options","nosniff"),("referrer-policy","no-referrer"),("cache-control","no-store"),("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'")] { response.headers_mut().insert(header::HeaderName::from_static(name),header::HeaderValue::from_static(val)); }
    response
}
fn router(app: App) -> Router {
    Router::new()
        .route(
            "/login",
            get(|| async { axum::response::Html(include_str!("../static/login.html")) }),
        )
        .route(
            "/setup",
            get(|| async { axum::response::Html(include_str!("../static/login.html")) }),
        )
        .route(
            "/activate",
            get(|| async { axum::response::Html(include_str!("../static/login.html")) }),
        )
        .route(
            "/admin",
            get(|| async { axum::response::Html(include_str!("../static/admin.html")) }),
        )
        .route("/api/auth/status", get(auth::status))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/activate", post(auth::activate))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/password", post(auth::change_password))
        .route("/api/auth/admin/catalog", get(auth::catalog))
        .route(
            "/api/auth/admin/accounts",
            get(auth::accounts).post(auth::invite),
        )
        .route("/api/auth/admin/accounts/{id}", put(auth::update_access))
        .route("/api/auth/admin/reissue", post(auth::reissue))
        .route("/api/auth/admin/audit", get(auth::audit_log))
        .route(
            "/",
            get(|| async { axum::response::Html(include_str!("../static/index.html")) }),
        )
        .route(
            "/patient",
            get(|| async { axum::response::Html(include_str!("../static/patient.html")) }),
        )
        .route(
            "/pharmacie",
            get(|| async { axum::response::Html(include_str!("../static/pharmacy.html")) }),
        )
        .route(
            "/domicile",
            get(|| async { axum::response::Html(include_str!("../static/nurse.html")) }),
        )
        .route("/api/patients/{id}/protection", put(protection::update))
        .route("/api/rights", post(rights::create))
        .route("/api/rights/{id}/updates", post(rights::update))
        .route("/api/nurses", post(nursing::create_nurse))
        .route("/api/nurse-options", get(nursing::nurse_options))
        .route("/api/nurse/{id}", get(nursing::nurse_view))
        .route("/api/patients/{id}/nurse", put(nursing::assign))
        .route("/api/visits", post(nursing::create_visit))
        .route("/api/nurse/{id}/visits/{vid}/report", post(nursing::report))
        .route("/api/nursing-reports/{id}/review", post(nursing::review))
        .route("/api/nursing-demo", post(nursing::demo))
        .route("/api/pharmacies", post(medication::create_pharmacy))
        .route("/api/pharmacy-options", get(medication::pharmacy_options))
        .route("/api/pharmacy/{id}", get(medication::pharmacy_view))
        .route(
            "/api/pharmacy/{id}/renewals/{rid}",
            put(medication::pharmacy_update),
        )
        .route("/api/patients/{id}/pharmacy", put(medication::assign))
        .route("/api/prescriptions", post(medication::create_prescription))
        .route(
            "/api/prescriptions/{id}/automation",
            put(medication::automation),
        )
        .route(
            "/api/prescriptions/{id}/request",
            post(medication::request_renewal),
        )
        .route("/api/renewals/{id}/decision", post(medication::decision))
        .route("/api/medication-demo", post(medication::demo))
        .route("/api/patient-options", get(patient_options))
        .route("/api/patient/{id}", get(patient_view))
        .route("/{name}", get(asset))
        .route("/api/state", get(get_state))
        .route("/api/patients", post(create_patient))
        .route("/api/programs", post(create_program))
        .route("/api/programs/{id}", put(revise_program))
        .route("/api/appointments", post(create_appointment))
        .route("/api/appointments/{id}", put(update_appointment))
        .route("/api/demo", post(seed))
        .layer(middleware::from_fn_with_state(app.clone(), auth::protect))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let default_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
    let path = std::env::var_os("PARCOURS_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_dir.join("parcours.sqlite"));
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let created = !parent.exists();
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        if created || parent == default_dir {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let app = open(&path)?;
    let scheduler = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let state = scheduler.clone();
            let result = tokio::task::spawn_blocking(move || {
                mutate(&state, |d| {
                    medication::reconcile(d, chrono::Local::now().date_naive())
                })
            })
            .await;
            if !matches!(result, Ok(Ok(_))) {
                eprintln!("Vérification des demandes automatiques échouée ; nouvel essai dans une minute.");
            }
        }
    });
    let bind = std::env::var("PARCOURS_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("Parcours Psy — http://127.0.0.1:8080\nPrototype local • données fictives uniquement\nArrêt : Ctrl+C");
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn patient() -> PatientInput {
        PatientInput {
            alias: "Démo A".into(),
            reference: "Dr Démo".into(),
            team: "CMP".into(),
        }
    }
    fn program(pid: String) -> ProgramInput {
        ProgramInput {
            patient_id: pid,
            author: "Dr Démo".into(),
            framework: "Test".into(),
            modalities: "Entretien".into(),
            start: "2026-10-01".into(),
            review: "2026-11-01".into(),
            status: "Actif".into(),
        }
    }
    #[test]
    fn failed_mutation_rolls_back() {
        let app = open(std::path::Path::new(":memory:")).unwrap();
        let result = mutate(&app, |d| {
            add_patient(d, patient())?;
            Err(invalid("échec"))
        });
        assert!(result.is_err());
        assert!(read(&app.db.lock().unwrap()).unwrap().patients.is_empty());
    }
    #[test]
    fn versions_preserve_original() {
        let mut d = Data::default();
        add_patient(&mut d, patient()).unwrap();
        let pid = d.patients[0].id.clone();
        add_program(&mut d, program(pid.clone())).unwrap();
        let id = d.programs[0].id.clone();
        let mut input = program(pid);
        input.modalities = "Nouvelle modalité".into();
        revise(&mut d, id, input).unwrap();
        assert_eq!(d.programs[0].versions.len(), 2);
        assert_eq!(d.programs[0].versions[0].modalities, "Entretien");
        assert_eq!(d.programs[0].versions[1].number, 2);
    }
    #[test]
    fn rejects_invalid_references_and_dates() {
        let mut d = Data::default();
        assert!(add_program(&mut d, program("absent".into())).is_err());
        let mut p = program("x".into());
        p.review = "2026-09-01".into();
        assert!(version(&p, 1).is_err());
        p.review = "2026-02-30".into();
        assert!(version(&p, 1).is_err());
    }
    #[tokio::test]
    async fn patient_view_contains_only_selected_patient_and_current_version() {
        let app = open(std::path::Path::new(":memory:")).unwrap();
        let data = mutate(&app, |d| {
            add_patient(d, patient())?;
            let first = d.patients[0].id.clone();
            add_program(d, program(first.clone()))?;
            revise(d, d.programs[0].id.clone(), program(first))?;
            add_patient(
                d,
                PatientInput {
                    alias: "Démo B".into(),
                    reference: "Autre référent".into(),
                    team: "Autre équipe".into(),
                },
            )?;
            add_program(d, program(d.patients[1].id.clone()))?;
            for p in &d.patients {
                d.appointments.push(Appointment {
                    id: id(),
                    patient_id: p.id.clone(),
                    date: "2026-11-01".into(),
                    time: "09:00".into(),
                    kind: "Test".into(),
                    professional: "Dr Test".into(),
                    place: "CMP".into(),
                    status: "Prévu".into(),
                });
            }
            Ok(())
        })
        .unwrap();
        let selected = data.patients[0].id.clone();
        let Json(result) = patient_view(State(app.clone()), Path(selected.clone()))
            .await
            .unwrap();
        assert_eq!(result.patient.id, selected);
        assert_eq!(result.programs.len(), 1);
        assert_eq!(result.programs[0].version.number, 2);
        assert_eq!(result.appointments.len(), 1);
        assert_eq!(result.appointments[0].patient_id, selected);
        assert!(patient_view(State(app), Path("inconnu".into()))
            .await
            .is_err());
    }
    #[test]
    fn duplicate_patients_rejected() {
        let mut d = Data::default();
        add_patient(&mut d, patient()).unwrap();
        assert!(add_patient(&mut d, patient()).is_err());
    }
}

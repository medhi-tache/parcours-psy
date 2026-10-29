//! Local demonstration workflow. Never creates a medical prescription or contacts a pharmacy.
use super::*;
#[derive(Clone, Serialize, Deserialize)]
pub struct Pharmacy {
    pub id: String,
    pub name: String,
    pub address: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Prescription {
    pub id: String,
    pub patient_id: String,
    pub label: String,
    pub reference: String,
    pub ends_on: String,
    pub request_on: String,
    pub auto_request: bool,
    pub revision: usize,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Renewal {
    pub id: String,
    pub prescription_id: String,
    pub patient_id: String,
    pub pharmacy_id: String,
    pub revision: usize,
    pub label: String,
    pub source_reference: String,
    pub status: String,
    pub created_at: String,
    pub decided_by: Option<String>,
    pub decision_at: Option<String>,
    pub new_reference: Option<String>,
    pub pharmacy_by: Option<String>,
    pub updated_at: String,
}
#[derive(Deserialize)]
pub struct PharmacyInput {
    name: String,
    address: String,
}
#[derive(Deserialize)]
pub struct Assignment {
    pharmacy_id: String,
}
#[derive(Deserialize)]
pub struct PrescriptionInput {
    patient_id: String,
    label: String,
    reference: String,
    ends_on: String,
    request_on: String,
    auto_request: bool,
}
#[derive(Deserialize)]
pub struct AutomationInput {
    enabled: bool,
}
#[derive(Deserialize)]
pub struct Decision {
    approved: bool,
    actor: String,
    confirmed: bool,
    reference: Option<String>,
    ends_on: Option<String>,
    request_on: Option<String>,
}
#[derive(Deserialize)]
pub struct PharmacyUpdate {
    status: String,
    actor: String,
}
#[derive(Serialize)]
pub struct PharmacyView {
    pub pharmacy: Pharmacy,
    pub renewals: Vec<PharmacyItem>,
}
#[derive(Serialize)]
pub struct PharmacyItem {
    pub renewal: Renewal,
    pub patient_alias: String,
}
fn pharmacy_exists(d: &Data, id: &str) -> Result<()> {
    if d.pharmacies.iter().any(|p| p.id == id) {
        Ok(())
    } else {
        Err(invalid("Pharmacie fictive inconnue."))
    }
}
pub fn create_request(d: &mut Data, pid: &str, now: NaiveDate, automatic: bool) -> Result<bool> {
    let p = d
        .prescriptions
        .iter()
        .find(|p| p.id == pid)
        .ok_or(invalid("Suivi d’ordonnance inconnu."))?
        .clone();
    if automatic && (!p.auto_request || date(&p.request_on)? > now) {
        return Ok(false);
    }
    if d.renewals.iter().any(|r| {
        r.prescription_id == pid
            && (r.revision == p.revision || !["Retirée", "Refusée"].contains(&r.status.as_str()))
    }) {
        return Ok(false);
    }
    let pharmacy_id = d
        .patients
        .iter()
        .find(|x| x.id == p.patient_id)
        .and_then(|x| x.pharmacy_id.clone())
        .ok_or(invalid(
            "Associez une pharmacie référente avant la demande.",
        ))?;
    pharmacy_exists(d, &pharmacy_id)?;
    let at = Utc::now().to_rfc3339();
    let r = Renewal {
        id: id(),
        prescription_id: p.id,
        patient_id: p.patient_id,
        pharmacy_id,
        revision: p.revision,
        label: p.label,
        source_reference: p.reference,
        status: "À valider".into(),
        created_at: at.clone(),
        decided_by: None,
        decision_at: None,
        new_reference: None,
        pharmacy_by: None,
        updated_at: at,
    };
    audit(
        d,
        if automatic {
            "Demande de renouvellement créée automatiquement (simulation)"
        } else {
            "Demande de renouvellement créée (simulation)"
        },
        r.id.clone(),
    );
    d.renewals.push(r);
    Ok(true)
}
pub fn reconcile(d: &mut Data, now: NaiveDate) -> Result<()> {
    let ids: Vec<String> = d
        .prescriptions
        .iter()
        .filter(|p| p.auto_request)
        .map(|p| p.id.clone())
        .collect();
    for id in ids {
        create_request(d, &id, now, true)?;
    }
    Ok(())
}
fn validate_cycle(ends: &str, request: &str) -> Result<()> {
    if date(request)? > date(ends)? {
        return Err(invalid(
            "La date de demande doit précéder ou égaler la fin renseignée.",
        ));
    }
    Ok(())
}
pub async fn create_pharmacy(
    State(app): State<App>,
    Json(i): Json<PharmacyInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        required(&i.name, "Nom fictif", 120)?;
        required(&i.address, "Adresse fictive", 250)?;
        let p = Pharmacy {
            id: id(),
            name: i.name.trim().into(),
            address: i.address.trim().into(),
        };
        audit(d, "Pharmacie fictive ajoutée", p.id.clone());
        d.pharmacies.push(p);
        Ok(())
    })?))
}
pub async fn assign(
    State(app): State<App>,
    Path(pid): Path<String>,
    Json(i): Json<Assignment>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        pharmacy_exists(d, &i.pharmacy_id)?;
        if d.renewals
            .iter()
            .any(|r| r.patient_id == pid && !["Retirée", "Refusée"].contains(&r.status.as_str()))
        {
            return Err(invalid(
                "Une demande est en cours. Terminez-la avant de changer de pharmacie référente.",
            ));
        }
        let p = d
            .patients
            .iter_mut()
            .find(|p| p.id == pid)
            .ok_or(invalid("Patient inconnu."))?;
        p.pharmacy_id = Some(i.pharmacy_id);
        audit(d, "Pharmacie référente renseignée (simulation)", pid);
        Ok(())
    })?))
}
pub async fn create_prescription(
    State(app): State<App>,
    Json(i): Json<PrescriptionInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        patient_exists(d, &i.patient_id)?;
        if d.patients
            .iter()
            .find(|p| p.id == i.patient_id)
            .and_then(|p| p.pharmacy_id.as_ref())
            .is_none()
        {
            return Err(invalid(
                "Renseignez d’abord la pharmacie référente du patient.",
            ));
        }
        required(&i.label, "Libellé du suivi", 200)?;
        required(&i.reference, "Référence de l’ordonnance fictive", 200)?;
        validate_cycle(&i.ends_on, &i.request_on)?;
        let p = Prescription {
            id: id(),
            patient_id: i.patient_id,
            label: i.label,
            reference: i.reference,
            ends_on: i.ends_on,
            request_on: i.request_on,
            auto_request: i.auto_request,
            revision: 1,
        };
        audit(d, "Suivi d’ordonnance fictive ajouté", p.id.clone());
        d.prescriptions.push(p);
        reconcile(d, chrono::Local::now().date_naive())
    })?))
}
pub async fn automation(
    State(app): State<App>,
    Path(pid): Path<String>,
    Json(i): Json<AutomationInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        let p = d
            .prescriptions
            .iter_mut()
            .find(|p| p.id == pid)
            .ok_or(invalid("Suivi inconnu."))?;
        p.auto_request = i.enabled;
        audit(
            d,
            if i.enabled {
                "Demandes automatiques activées"
            } else {
                "Demandes automatiques suspendues"
            },
            pid,
        );
        reconcile(d, chrono::Local::now().date_naive())
    })?))
}
pub async fn request_renewal(
    State(app): State<App>,
    Path(pid): Path<String>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        if !create_request(d, &pid, chrono::Local::now().date_naive(), false)? {
            return Err(invalid(
                "Une demande existe déjà pour ce cycle ou reste en cours.",
            ));
        }
        Ok(())
    })?))
}
pub fn decide(d: &mut Data, rid: &str, i: Decision) -> Result<()> {
    required(&i.actor, "Prescripteur déclaré", 120)?;
    if !i.confirmed {
        return Err(invalid("La confirmation du prescripteur est requise."));
    }
    let index = d
        .renewals
        .iter()
        .position(|r| r.id == rid)
        .ok_or(invalid("Demande inconnue."))?;
    let r = d.renewals[index].clone();
    if r.status != "À valider" {
        return Err(invalid("Cette demande a déjà été traitée."));
    }
    if i.approved {
        let reference = i.reference.ok_or(invalid("Nouvelle référence requise."))?;
        required(&reference, "Nouvelle ordonnance fictive", 200)?;
        let ends = i.ends_on.ok_or(invalid("Nouvelle date de fin requise."))?;
        let request = i
            .request_on
            .ok_or(invalid("Nouvelle date de demande requise."))?;
        validate_cycle(&ends, &request)?;
        let p = d
            .prescriptions
            .iter_mut()
            .find(|p| p.id == r.prescription_id)
            .ok_or(invalid("Suivi inconnu."))?;
        if p.revision != r.revision {
            return Err(invalid("Le suivi a changé. Réexaminez la demande."));
        }
        if date(&ends)? <= date(&p.ends_on)? || date(&request)? <= chrono::Local::now().date_naive()
        {
            return Err(invalid("La nouvelle fin doit suivre la précédente et la prochaine demande doit être dans le futur."));
        }
        p.reference = reference.clone();
        p.ends_on = ends;
        p.request_on = request;
        p.revision += 1;
        d.renewals[index].new_reference = Some(reference);
        d.renewals[index].status = "À préparer".into();
    } else {
        d.renewals[index].status = "Refusée".into();
    }
    let at = Utc::now().to_rfc3339();
    d.renewals[index].decided_by = Some(i.actor);
    d.renewals[index].decision_at = Some(at.clone());
    d.renewals[index].updated_at = at;
    audit(
        d,
        if i.approved {
            "Validation déclarée du prescripteur — file pharmacie locale (simulation)"
        } else {
            "Demande refusée par le prescripteur déclaré (simulation)"
        },
        rid.into(),
    );
    Ok(())
}
pub async fn decision(
    State(app): State<App>,
    Path(rid): Path<String>,
    Json(i): Json<Decision>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| decide(d, &rid, i))?))
}
pub fn pharmacy_status(d: &mut Data, fid: &str, rid: &str, i: PharmacyUpdate) -> Result<()> {
    required(&i.actor, "Pharmacien déclaré", 120)?;
    let r = d
        .renewals
        .iter_mut()
        .find(|r| r.id == rid && r.pharmacy_id == fid)
        .ok_or(invalid("Demande inconnue pour cette pharmacie."))?;
    let allowed = matches!(
        (r.status.as_str(), i.status.as_str()),
        ("À préparer", "En préparation") | ("En préparation", "Prête") | ("Prête", "Retirée")
    );
    if !allowed || r.decision_at.is_none() || r.new_reference.is_none() {
        return Err(invalid("Transition impossible : validation du prescripteur puis confirmation de la pharmacie requises."));
    }
    r.status = i.status.clone();
    r.pharmacy_by = Some(i.actor);
    r.updated_at = Utc::now().to_rfc3339();
    audit(
        d,
        &format!("Pharmacie : {} (simulation)", i.status),
        rid.into(),
    );
    Ok(())
}
pub async fn pharmacy_update(
    State(app): State<App>,
    Path((fid, rid)): Path<(String, String)>,
    Json(i): Json<PharmacyUpdate>,
) -> Result<Json<PharmacyView>> {
    mutate(&app, |d| pharmacy_status(d, &fid, &rid, i))?;
    pharmacy_view(State(app), Path(fid)).await
}
pub async fn pharmacy_view(
    State(app): State<App>,
    Path(fid): Path<String>,
) -> Result<Json<PharmacyView>> {
    let conn = app.db.lock().map_err(internal)?;
    let d = read(&conn)?;
    let pharmacy = d
        .pharmacies
        .iter()
        .find(|p| p.id == fid)
        .cloned()
        .ok_or(invalid("Pharmacie inconnue."))?;
    let renewals = d
        .renewals
        .iter()
        .filter(|r| r.pharmacy_id == fid && !["À valider", "Refusée"].contains(&r.status.as_str()))
        .map(|r| PharmacyItem {
            renewal: r.clone(),
            patient_alias: d
                .patients
                .iter()
                .find(|p| p.id == r.patient_id)
                .map(|p| p.alias.clone())
                .unwrap_or_default(),
        })
        .collect();
    Ok(Json(PharmacyView { pharmacy, renewals }))
}
pub async fn pharmacy_options(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<auth::Principal>,
) -> Result<Json<Vec<Pharmacy>>> {
    let conn = app.db.lock().map_err(internal)?;
    Ok(Json(
        read(&conn)?
            .pharmacies
            .into_iter()
            .filter(|x| p.subject_id.as_ref() == Some(&x.id))
            .collect(),
    ))
}
pub async fn demo(State(app): State<App>) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        if !d.prescriptions.is_empty() {
            return Err(invalid("Des suivis existent déjà."));
        }
        if d.patients.is_empty() {
            return Err(invalid("Ajoutez d’abord un patient fictif."));
        }
        let f = Pharmacy {
            id: id(),
            name: "Pharmacie Démo du Parc".into(),
            address: "Adresse fictive — espace local".into(),
        };
        let fid = f.id.clone();
        d.pharmacies.push(f);
        let today = chrono::Local::now().date_naive();
        let patients = d
            .patients
            .iter_mut()
            .take(2)
            .map(|p| {
                p.pharmacy_id = Some(fid.clone());
                p.id.clone()
            })
            .collect::<Vec<_>>();
        for pid in patients {
            d.prescriptions.push(Prescription {
                id: id(),
                patient_id: pid,
                label: "Traitement fictif — exemple sans molécule ni posologie".into(),
                reference: "ORD-DEMO-001".into(),
                ends_on: (today + chrono::Duration::days(7)).to_string(),
                request_on: today.to_string(),
                auto_request: true,
                revision: 1,
            });
        }
        audit(
            d,
            "Exemples de circuit ordonnance chargés (simulation)",
            "demo".into(),
        );
        reconcile(d, today)
    })?))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Data {
        let mut d = Data::default();
        add_patient(
            &mut d,
            PatientInput {
                alias: "A".into(),
                reference: "Dr".into(),
                team: "CMP".into(),
            },
        )
        .unwrap();
        d.pharmacies.push(Pharmacy {
            id: "f".into(),
            name: "Démo".into(),
            address: "Test".into(),
        });
        d.patients[0].pharmacy_id = Some("f".into());
        d.prescriptions.push(Prescription {
            id: "p".into(),
            patient_id: d.patients[0].id.clone(),
            label: "Fictif".into(),
            reference: "ord1".into(),
            ends_on: "2030-01-10".into(),
            request_on: "2030-01-01".into(),
            auto_request: true,
            revision: 1,
        });
        d
    }
    #[test]
    fn automation_is_due_only_and_idempotent() {
        let mut d = fixture();
        reconcile(&mut d, date("2029-12-31").unwrap()).unwrap();
        assert!(d.renewals.is_empty());
        reconcile(&mut d, date("2030-01-01").unwrap()).unwrap();
        reconcile(&mut d, date("2030-01-02").unwrap()).unwrap();
        assert_eq!(d.renewals.len(), 1);
        assert_eq!(d.renewals[0].status, "À valider");
        assert_eq!(d.prescriptions[0].reference, "ord1");
    }
    #[test]
    fn pharmacy_cannot_bypass_doctor_or_mark_ready_directly() {
        let mut d = fixture();
        create_request(&mut d, "p", date("2030-01-01").unwrap(), false).unwrap();
        let rid = d.renewals[0].id.clone();
        assert!(pharmacy_status(
            &mut d,
            "f",
            &rid,
            PharmacyUpdate {
                status: "Prête".into(),
                actor: "Ph".into()
            }
        )
        .is_err());
        assert!(decide(
            &mut d,
            &rid,
            Decision {
                approved: true,
                actor: "Dr".into(),
                confirmed: false,
                reference: Some("ord2".into()),
                ends_on: Some("2030-02-10".into()),
                request_on: Some("2030-02-01".into())
            }
        )
        .is_err());
        decide(
            &mut d,
            &rid,
            Decision {
                approved: true,
                actor: "Dr".into(),
                confirmed: true,
                reference: Some("ord2".into()),
                ends_on: Some("2030-02-10".into()),
                request_on: Some("2030-02-01".into()),
            },
        )
        .unwrap();
        assert!(pharmacy_status(
            &mut d,
            "f",
            &rid,
            PharmacyUpdate {
                status: "Prête".into(),
                actor: "Ph".into()
            }
        )
        .is_err());
        for status in ["En préparation", "Prête", "Retirée"] {
            pharmacy_status(
                &mut d,
                "f",
                &rid,
                PharmacyUpdate {
                    status: status.into(),
                    actor: "Ph".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(d.prescriptions[0].revision, 2);
    }
    #[test]
    fn suspended_automation_creates_nothing() {
        let mut d = fixture();
        d.prescriptions[0].auto_request = false;
        reconcile(&mut d, date("2030-01-20").unwrap()).unwrap();
        assert!(d.renewals.is_empty());
    }
    #[test]
    fn old_database_loads_with_empty_medication_fields() {
        let d: Data =
            serde_json::from_str(r#"{"patients":[],"programs":[],"appointments":[],"history":[]}"#)
                .unwrap();
        assert!(d.renewals.is_empty());
    }
}

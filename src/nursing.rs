//! Home nursing coordination for fictitious data, with declared actors and no clinical decisions.
use super::*;
#[derive(Clone, Serialize, Deserialize)]
pub struct Nurse {
    pub id: String,
    pub name: String,
    pub team: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Visit {
    pub id: String,
    pub patient_id: String,
    pub nurse_id: String,
    pub date: String,
    pub time: String,
    pub purpose: String,
    pub status: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub visit_id: String,
    pub patient_id: String,
    pub nurse_id: String,
    pub actor: String,
    pub at: String,
    pub availability: String,
    pub intake: String,
    pub note: String,
    pub needs_review: bool,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
}
#[derive(Deserialize)]
pub struct NurseInput {
    name: String,
    team: String,
}
#[derive(Deserialize)]
pub struct Assign {
    nurse_id: String,
}
#[derive(Deserialize)]
pub struct VisitInput {
    patient_id: String,
    date: String,
    time: String,
    purpose: String,
}
#[derive(Deserialize)]
pub struct ReportInput {
    status: String,
    actor: String,
    availability: String,
    intake: String,
    note: String,
    needs_review: bool,
}
#[derive(Deserialize)]
pub struct ReviewInput {
    actor: String,
}
#[derive(Serialize)]
pub struct NurseView {
    pub nurse: Nurse,
    pub patients: Vec<NursingPatient>,
    pub visits: Vec<Visit>,
    pub reports: Vec<Report>,
}
#[derive(Serialize)]
pub struct NursingPatient {
    pub patient: Patient,
    pub pharmacy: Option<Pharmacy>,
    pub prescriptions: Vec<Prescription>,
    pub renewals: Vec<Renewal>,
}
pub async fn create_nurse(State(app): State<App>, Json(i): Json<NurseInput>) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        required(&i.name, "Nom fictif", 120)?;
        required(&i.team, "Cabinet / équipe", 150)?;
        let n = Nurse {
            id: id(),
            name: i.name,
            team: i.team,
        };
        audit(
            d,
            "Professionnel à domicile ajouté (simulation)",
            n.id.clone(),
        );
        d.nurses.push(n);
        Ok(())
    })?))
}
pub async fn assign(
    State(app): State<App>,
    Path(pid): Path<String>,
    Json(i): Json<Assign>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        if !d.nurses.iter().any(|n| n.id == i.nurse_id) {
            return Err(invalid("Professionnel inconnu."));
        }
        if d.visits
            .iter()
            .any(|v| v.patient_id == pid && v.status == "Prévue")
        {
            return Err(invalid("Des visites sont prévues. Terminez ou annulez ces visites avant de changer le référent."));
        }
        let p = d
            .patients
            .iter_mut()
            .find(|p| p.id == pid)
            .ok_or(invalid("Patient inconnu."))?;
        p.nurse_id = Some(i.nurse_id);
        audit(d, "Référent à domicile renseigné (simulation)", pid);
        Ok(())
    })?))
}
pub async fn create_visit(State(app): State<App>, Json(i): Json<VisitInput>) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        date(&i.date)?;
        NaiveTime::parse_from_str(&i.time, "%H:%M").map_err(|_| invalid("Heure invalide."))?;
        required(&i.purpose, "Objet de visite", 300)?;
        let nurse = d
            .patients
            .iter()
            .find(|p| p.id == i.patient_id)
            .and_then(|p| p.nurse_id.clone())
            .ok_or(invalid("Associez un référent à domicile au patient."))?;
        if d.visits.iter().any(|v| {
            v.nurse_id == nurse && v.date == i.date && v.time == i.time && v.status == "Prévue"
        }) {
            return Err(invalid("Ce professionnel a déjà une visite à cet horaire."));
        }
        let v = Visit {
            id: id(),
            patient_id: i.patient_id,
            nurse_id: nurse,
            date: i.date,
            time: i.time,
            purpose: i.purpose,
            status: "Prévue".into(),
        };
        audit(d, "Visite à domicile planifiée", v.id.clone());
        d.visits.push(v);
        Ok(())
    })?))
}
pub fn record(d: &mut Data, nid: &str, vid: &str, i: ReportInput) -> Result<()> {
    required(&i.actor, "Intervenant déclaré", 120)?;
    required(&i.note, "Observation de visite", 2000)?;
    if !["Réalisée", "Patient absent", "Annulée"].contains(&i.status.as_str())
        || ![
            "Non vérifiée",
            "Disponibilité constatée",
            "Indisponibilité signalée",
        ]
        .contains(&i.availability.as_str())
        || ![
            "Non évaluée",
            "Prise constatée",
            "Non prise / refus signalé",
        ]
        .contains(&i.intake.as_str())
    {
        return Err(invalid("État ou observation invalide."));
    }
    if i.status != "Réalisée" && (i.availability != "Non vérifiée" || i.intake != "Non évaluée")
    {
        return Err(invalid(
            "Une visite non réalisée ne permet pas de constater une disponibilité ou une prise.",
        ));
    }
    let v = d
        .visits
        .iter_mut()
        .find(|v| v.id == vid && v.nurse_id == nid)
        .ok_or(invalid("Visite inconnue pour ce professionnel."))?;
    if v.status != "Prévue" {
        return Err(invalid("Cette visite a déjà un compte rendu."));
    }
    if date(&v.date)? > chrono::Local::now().date_naive() && i.status != "Annulée" {
        return Err(invalid(
            "Une visite future ne peut pas être déclarée réalisée ou absente.",
        ));
    }
    v.status = i.status.clone();
    let patient_id = v.patient_id.clone();
    let review = i.needs_review
        || i.status == "Patient absent"
        || i.availability == "Indisponibilité signalée"
        || i.intake == "Non prise / refus signalé";
    let r = Report {
        id: id(),
        visit_id: vid.into(),
        patient_id,
        nurse_id: nid.into(),
        actor: i.actor,
        at: Utc::now().to_rfc3339(),
        availability: i.availability,
        intake: i.intake,
        note: i.note,
        needs_review: review,
        reviewed_by: None,
        reviewed_at: None,
    };
    audit(
        d,
        if review {
            "Compte rendu à domicile — à examiner par l’équipe (local)"
        } else {
            "Compte rendu de visite à domicile enregistré"
        },
        r.id.clone(),
    );
    d.nursing_reports.push(r);
    Ok(())
}
pub async fn report(
    State(app): State<App>,
    Path((nid, vid)): Path<(String, String)>,
    Json(i): Json<ReportInput>,
) -> Result<Json<NurseView>> {
    mutate(&app, |d| record(d, &nid, &vid, i))?;
    nurse_view(State(app), Path(nid)).await
}
pub async fn review(
    State(app): State<App>,
    Path(rid): Path<String>,
    Json(i): Json<ReviewInput>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        required(&i.actor, "Professionnel ayant pris connaissance", 120)?;
        let r = d
            .nursing_reports
            .iter_mut()
            .find(|r| r.id == rid)
            .ok_or(invalid("Compte rendu inconnu."))?;
        if r.reviewed_at.is_some() {
            return Err(invalid("La prise de connaissance est déjà enregistrée."));
        }
        r.reviewed_by = Some(i.actor);
        r.reviewed_at = Some(Utc::now().to_rfc3339());
        audit(
            d,
            "Prise de connaissance du compte rendu déclarée par l’équipe",
            rid,
        );
        Ok(())
    })?))
}
pub async fn nurse_options(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<auth::Principal>,
) -> Result<Json<Vec<Nurse>>> {
    let conn = app.db.lock().map_err(internal)?;
    Ok(Json(
        read(&conn)?
            .nurses
            .into_iter()
            .filter(|x| p.subject_id.as_ref() == Some(&x.id))
            .collect(),
    ))
}
pub async fn nurse_view(
    State(app): State<App>,
    Path(nid): Path<String>,
) -> Result<Json<NurseView>> {
    let conn = app.db.lock().map_err(internal)?;
    let d = read(&conn)?;
    let nurse = d
        .nurses
        .iter()
        .find(|n| n.id == nid)
        .cloned()
        .ok_or(invalid("Professionnel inconnu."))?;
    let patients = d
        .patients
        .iter()
        .filter(|p| p.nurse_id.as_ref() == Some(&nid))
        .map(|p| NursingPatient {
            patient: p.clone(),
            pharmacy: d
                .pharmacies
                .iter()
                .find(|f| p.pharmacy_id.as_ref() == Some(&f.id))
                .cloned(),
            prescriptions: d
                .prescriptions
                .iter()
                .filter(|x| x.patient_id == p.id)
                .cloned()
                .collect(),
            renewals: d
                .renewals
                .iter()
                .filter(|x| x.patient_id == p.id)
                .cloned()
                .collect(),
        })
        .collect();
    let visits = d.visits.into_iter().filter(|v| v.nurse_id == nid).collect();
    let reports = d
        .nursing_reports
        .into_iter()
        .filter(|r| r.nurse_id == nid)
        .collect();
    Ok(Json(NurseView {
        nurse,
        patients,
        visits,
        reports,
    }))
}
pub async fn demo(State(app): State<App>) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| {
        if !d.nurses.is_empty() {
            return Err(invalid("Des professionnels à domicile existent déjà."));
        }
        if d.patients.is_empty() {
            return Err(invalid("Ajoutez d’abord des patients fictifs."));
        }
        let n = Nurse {
            id: id(),
            name: "Camille Démo · IDE".into(),
            team: "Cabinet infirmier Démo".into(),
        };
        let nid = n.id.clone();
        d.nurses.push(n);
        let now = chrono::Local::now().date_naive();
        for (i, p) in d.patients.iter_mut().take(2).enumerate() {
            p.nurse_id = Some(nid.clone());
            d.visits.push(Visit {
                id: id(),
                patient_id: p.id.clone(),
                nurse_id: nid.clone(),
                date: now.to_string(),
                time: format!("{:02}:30", 8 + i),
                purpose: "Visite de suivi prévue par l’équipe — exemple fictif".into(),
                status: "Prévue".into(),
            });
        }
        audit(d, "Exemples de soins à domicile chargés", "demo".into());
        Ok(())
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
        d.nurses.push(Nurse {
            id: "n".into(),
            name: "IDE".into(),
            team: "Test".into(),
        });
        d.patients[0].nurse_id = Some("n".into());
        d.visits.push(Visit {
            id: "v".into(),
            patient_id: d.patients[0].id.clone(),
            nurse_id: "n".into(),
            date: chrono::Local::now().date_naive().to_string(),
            time: "09:00".into(),
            purpose: "Test".into(),
            status: "Prévue".into(),
        });
        d
    }
    fn input() -> ReportInput {
        ReportInput {
            status: "Réalisée".into(),
            actor: "IDE Démo".into(),
            availability: "Indisponibilité signalée".into(),
            intake: "Non évaluée".into(),
            note: "Exemple fictif à examiner".into(),
            needs_review: false,
        }
    }
    #[test]
    fn report_flags_observed_difficulty_without_changing_prescription() {
        let mut d = fixture();
        record(&mut d, "n", "v", input()).unwrap();
        assert!(d.nursing_reports[0].needs_review);
        assert_eq!(d.visits[0].status, "Réalisée");
        assert!(d.prescriptions.is_empty());
        assert!(record(&mut d, "n", "v", input()).is_err());
    }
    #[test]
    fn incorrect_nurse_and_future_visit_rejected() {
        let mut d = fixture();
        assert!(record(&mut d, "other", "v", input()).is_err());
        d.visits[0].date =
            (chrono::Local::now().date_naive() + chrono::Duration::days(1)).to_string();
        assert!(record(&mut d, "n", "v", input()).is_err());
    }
    #[test]
    fn absent_visit_cannot_claim_intake() {
        let mut d = fixture();
        let mut i = input();
        i.status = "Patient absent".into();
        i.intake = "Prise constatée".into();
        assert!(record(&mut d, "n", "v", i).is_err());
    }
    #[tokio::test]
    async fn nurse_view_scopes_assigned_patients() {
        let app = open(std::path::Path::new(":memory:")).unwrap();
        mutate(&app, |d| {
            *d = fixture();
            add_patient(
                d,
                PatientInput {
                    alias: "B".into(),
                    reference: "Dr".into(),
                    team: "CMP".into(),
                },
            )?;
            Ok(())
        })
        .unwrap();
        let Json(view) = nurse_view(State(app), Path("n".into())).await.unwrap();
        assert_eq!(view.patients.len(), 1);
        assert_eq!(view.visits.len(), 1);
    }
}

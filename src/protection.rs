//! Administrative record of curatorship. Never grants a curator access or decision-making rights.
use super::*;
#[derive(Clone, Serialize, Deserialize)]
pub struct Protection {
    pub id: String,
    pub patient_id: String,
    pub versions: Vec<ProtectionVersion>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ProtectionVersion {
    pub number: usize,
    pub kind: String,
    pub status: String,
    pub starts_on: String,
    pub ends_on: Option<String>,
    pub judgment_reference: String,
    pub scope: String,
    pub personal_assistance: String,
    pub curator_name: String,
    pub organization: String,
    pub phone: String,
    pub email: String,
    pub recorded_at: String,
    pub actor_id: String,
    pub actor_name: String,
}
#[derive(Deserialize)]
pub struct Input {
    pub kind: String,
    pub status: String,
    pub starts_on: String,
    pub ends_on: Option<String>,
    pub judgment_reference: String,
    pub scope: String,
    pub personal_assistance: String,
    pub curator_name: String,
    #[serde(default)]
    pub organization: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub email: String,
}
pub fn current<'a>(d: &'a Data, pid: &str) -> Option<&'a Protection> {
    d.protections.iter().find(|p| p.patient_id == pid)
}
pub fn record(d: &mut Data, p: &auth::Principal, pid: &str, input: Input) -> Result<()> {
    if !["doctor", "team"].contains(&p.role.as_str()) || !p.patient_ids.iter().any(|id| id == pid) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Mesure inaccessible pour ce compte.".into(),
        ));
    }
    patient_exists(d, pid)?;
    if ![
        "Curatelle simple",
        "Curatelle renforcée",
        "Curatelle aménagée",
    ]
    .contains(&input.kind.as_str())
        || !["En cours", "Terminée"].contains(&input.status.as_str())
        || ![
            "À vérifier dans la décision",
            "Prévue par la décision",
            "Non prévue par la décision",
        ]
        .contains(&input.personal_assistance.as_str())
    {
        return Err(invalid("Type, état ou périmètre d’assistance invalide."));
    }
    let start = date(&input.starts_on)?;
    let end = input.ends_on.filter(|s| !s.trim().is_empty());
    if let Some(ref e) = end {
        if date(e)? < start {
            return Err(invalid(
                "La fin de la mesure ne peut pas précéder son début.",
            ));
        }
    }
    if input.status == "Terminée" && end.is_none() {
        return Err(invalid(
            "Renseignez la date de fin figurant dans la décision.",
        ));
    }
    required(
        &input.judgment_reference,
        "Référence de la décision fictive",
        200,
    )?;
    required(&input.scope, "Périmètre de la mesure", 2000)?;
    required(&input.curator_name, "Curateur ou curatrice", 120)?;
    if input.organization.chars().count() > 200
        || input.phone.chars().count() > 50
        || input.email.chars().count() > 254
    {
        return Err(invalid("Coordonnées trop longues."));
    }
    let existing = d.protections.iter().position(|x| x.patient_id == pid);
    let number = existing
        .map(|i| d.protections[i].versions.len() + 1)
        .unwrap_or(1);
    let v = ProtectionVersion {
        number,
        kind: input.kind,
        status: input.status,
        starts_on: input.starts_on,
        ends_on: end,
        judgment_reference: input.judgment_reference.trim().into(),
        scope: input.scope.trim().into(),
        personal_assistance: input.personal_assistance,
        curator_name: input.curator_name.trim().into(),
        organization: input.organization.trim().into(),
        phone: input.phone.trim().into(),
        email: input.email.trim().into(),
        recorded_at: Utc::now().to_rfc3339(),
        actor_id: p.id.clone(),
        actor_name: p.name.clone(),
    };
    let entity = if let Some(i) = existing {
        d.protections[i].versions.push(v);
        d.protections[i].id.clone()
    } else {
        let id = id();
        d.protections.push(Protection {
            id: id.clone(),
            patient_id: pid.into(),
            versions: vec![v],
        });
        id
    };
    audit(
        d,
        "Mesure de curatelle renseignée — aucun accès curateur attribué",
        entity,
    );
    Ok(())
}
pub async fn update(
    State(app): State<App>,
    Path(pid): Path<String>,
    axum::Extension(p): axum::Extension<auth::Principal>,
    Json(input): Json<Input>,
) -> Result<Json<Data>> {
    Ok(Json(mutate(&app, |d| record(d, &p, &pid, input))?))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn user() -> auth::Principal {
        auth::Principal {
            id: "u".into(),
            username: "test".into(),
            name: "Équipe de test".into(),
            role: "team".into(),
            subject_id: None,
            patient_ids: vec!["p".into()],
            enabled: true,
            activated: true,
        }
    }
    fn data() -> Data {
        let mut d = Data::default();
        d.patients.push(Patient {
            id: "p".into(),
            alias: "Patient fictif".into(),
            reference: "Dr Fictif".into(),
            team: "CMP".into(),
            pharmacy_id: None,
            nurse_id: None,
        });
        d
    }
    fn input() -> Input {
        Input {
            kind: "Curatelle renforcée".into(),
            status: "En cours".into(),
            starts_on: "2026-01-01".into(),
            ends_on: Some("2030-01-01".into()),
            judgment_reference: "REF-FICTIVE".into(),
            scope: "Périmètre à vérifier dans la décision.".into(),
            personal_assistance: "À vérifier dans la décision".into(),
            curator_name: "Curatrice fictive".into(),
            organization: "Service fictif".into(),
            phone: String::new(),
            email: String::new(),
        }
    }
    #[test]
    fn preserves_versions_and_does_not_change_patient_program_or_rights() {
        let mut d = data();
        let before = serde_json::to_value(&d.patients).unwrap();
        record(&mut d, &user(), "p", input()).unwrap();
        let mut next = input();
        next.kind = "Curatelle aménagée".into();
        record(&mut d, &user(), "p", next).unwrap();
        assert_eq!(d.protections[0].versions.len(), 2);
        assert_eq!(d.protections[0].versions[0].kind, "Curatelle renforcée");
        assert_eq!(serde_json::to_value(&d.patients).unwrap(), before);
        assert!(d.programs.is_empty() && d.rights_requests.is_empty());
    }
    #[test]
    fn rejects_invalid_dates_and_unassigned_or_non_staff_accounts() {
        let mut d = data();
        let mut i = input();
        i.ends_on = Some("2025-01-01".into());
        assert!(record(&mut d, &user(), "p", i).is_err());
        let mut p = user();
        p.patient_ids.clear();
        assert!(record(&mut d, &p, "p", input()).is_err());
        p.role = "patient".into();
        p.patient_ids = vec!["p".into()];
        assert!(record(&mut d, &p, "p", input()).is_err());
        assert!(d.protections.is_empty());
    }
}

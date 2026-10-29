# Parcours Psy — Next.js + API Rust

Application de coordination des programmes de soins psychiatriques sans consentement, avec une interface React/Next.js pour ordinateur et mobile et une API Rust indépendante.

**Développement local uniquement.** Aucun service en ligne n’a été publié. Les données et les parcours de démonstration sont fictifs.

## Architecture

```text
Navigateur ordinateur / mobile
          │
          ▼
Interface Next.js — comptes et espaces selon le rôle
          │ /api/* : relais côté serveur, sans cache
          ▼
API Rust / Axum — authentification et autorisations
          │
          ▼
SQLite — données, comptes, sessions et historique
```

L’adresse de l’API se configure dans `frontend/.env.local` avec `RUST_API_URL`. Elle reste côté serveur Next.js. Les écrans React ne téléchargent pas un état global avant de le filtrer : l’API contrôle le rôle, le dossier et les actions autorisés avant de renvoyer les données. Les réponses aux mutations sont également filtrées.

L’interface est adaptée aux navigateurs sur Windows, macOS, Linux et mobile. Cette version n’est pas un installateur natif Windows/macOS/Linux et ne contient pas encore de client mobile natif. L’architecture retenue suite à la demande est une interface Next.js connectée à une API Rust séparée.

## Lancer sur cet ordinateur

Ouvrir **http://127.0.0.1:3000** lorsque les deux services sont lancés.

Pour relancer :

- macOS : double-cliquer `Lancer.command` ;
- Windows : ouvrir `Lancer-Windows.cmd` ;
- Linux : exécuter `./Lancer-Linux.sh` ;
- ou, sur les trois systèmes : `node scripts/start.mjs`.

Les lanceurs utilisent Node.js, Next.js et Rust, démarrent les deux services en local et s’arrêtent avec `Ctrl+C`. Ils refusent de réutiliser silencieusement un serveur inconnu déjà présent sur les ports 3000 ou 8080. Laisser la fenêtre de lancement ouverte.

Les outils et dépendances sont installés sur ce Mac. Sur une nouvelle machine, installer Rust et Node.js 20.9 ou supérieur, puis depuis le dossier du projet :

```sh
cargo build --locked
cd frontend
npm ci
npm run build
```

Ces étapes nécessitent le réseau uniquement pour télécharger les dépendances publiques. Après modification de l’interface, exécuter de nouveau `npm run build` avant un lancement en production locale. En développement, on peut lancer séparément :

```sh
# Terminal 1, à la racine du projet
cargo run --locked --offline

# Terminal 2, dans frontend
npm run dev
```

La page historique servie directement par Rust reste accessible sur le port 8080 pour compatibilité. L’entrée principale de l’application est désormais Next.js sur le port 3000.

## Premier compte et activation

1. Ouvrir l’application : lorsqu’aucun compte n’existe, elle propose **Créer le premier administrateur**.
2. Choisir soi-même le nom, l’identifiant et le mot de passe (12 caractères minimum). Aucun compte ou mot de passe par défaut n’est livré.
3. Dans l’administration, créer les comptes et leurs affectations.
4. Remettre au titulaire le code d’activation affiché une seule fois, valable 24 heures. La remise du code est manuelle ; aucun courriel ou SMS n’est envoyé.
5. Le titulaire utilise **J’ai un code d’activation** et choisit lui-même son mot de passe.

Un nouveau code peut être généré pour un compte non encore activé. L’ancien devient inutilisable. Le rôle d’un compte reste fixe ; ses affectations et son activation peuvent être modifiées. Une modification des accès révoque les sessions de ce compte. Il est impossible de désactiver son propre compte ou le dernier administrateur actif.

## Rôles et accès

| Rôle | Accès et actions |
| --- | --- |
| Administrateur | Comptes, affectations, métadonnées des profils et initialisation des exemples. Aucun accès aux décisions cliniques. |
| Médecin | Dossiers explicitement attribués, programmes/versionnement, suivi des ordonnances et validation des demandes de renouvellement. |
| Équipe de soins | Dossiers attribués, rendez-vous, visites, référents, prise de connaissance des observations et accompagnement des demandes de contestation. Aucune validation de renouvellement. |
| Patient | Son dossier, ses programmes actuels, rendez-vous, visites, pharmacie, demandes d’examen d’ordonnance et demandes d’accompagnement à la contestation. |
| Infirmier à domicile | Son profil professionnel, ses patients associés, ses visites, observations et états de préparation en pharmacie. |
| Pharmacie | Son officine et les demandes validées qui lui sont destinées ; confirmations de préparation et de retrait. |

Les professionnels de l’équipe ont accès à l’annuaire partagé des référents infirmiers et des pharmacies pour organiser les affectations. Les comptes patients, infirmiers et pharmaciens ne disposent plus de sélecteurs donnant accès à d’autres profils.

Les noms d’auteurs des nouvelles versions, validations, préparations et comptes rendus sont liés au compte connecté côté serveur. Les valeurs d’identité envoyées par le navigateur ne font pas autorité. Le journal des comptes consigne les connexions et les actions sans enregistrer de mots de passe, codes d’activation ou contenus de formulaire.

## Fonctions métier

- Tableau de bord des dossiers attribués, recherche et filtre des programmes.
- Programmes versionnés : cadre saisi, modalités, auteur, dates et statut ; conservation des versions précédentes.
- Agenda des rendez-vous et suivi des états prévu/réalisé/absent/annulé.
- Pharmacie et référent infirmier associés au patient.
- Planification des visites à domicile, observations de disponibilité et de prise uniquement lorsqu’elles ont été constatées.
- Signalements locaux à examiner et prise de connaissance par l’équipe.
- Historique et export JSON des données accessibles au professionnel.

Les cadres juridiques, modalités de soins, dates et références sont saisis par les professionnels. Aucun délai légal, diagnostic, dosage ou décision clinique n’est calculé par le logiciel.

## Renouvellement et préparation des médicaments

Un suivi d’ordonnance existante contient un libellé, une référence fictive, une date de fin et une date de demande. Le backend examine les échéances chaque minute tant qu’il reste lancé, y compris si le navigateur est fermé. Il crée une **demande à valider**, jamais une prescription médicale. Les demandes ne sont pas dupliquées pour un même cycle.

Le médecin affecté au dossier peut enregistrer une décision, une référence de nouvelle ordonnance fictive et les prochaines dates. Après cette validation, la pharmacie concernée voit la demande. Les transitions sont confirmées manuellement : **À préparer → En préparation → Prête → Retirée**. Le statut « Prête » n’est pas déduit d’un délai ni d’un calcul de stock.

Le patient et l’infirmier retrouvent l’avancement. La simulation ne génère pas d’ordonnance signée, ne contacte aucune officine réelle et n’effectue aucune délivrance réglementaire. Aucun stock, transport, remboursement ou facturation n’est géré.

## Contestation, avocat et avis psychiatrique indépendant

Dans **Mes droits et contestation**, le patient peut demander un accompagnement pour contacter un avocat de son choix et/ou discuter d’un avis psychiatrique indépendant ou d’une contre-expertise. L’enregistrement ne nécessite pas l’accord du prescripteur.

La demande conserve le programme et sa version au moment de l’enregistrement. Le patient confirme que le message sera visible par l’équipe affectée à son dossier. Il est averti de ne pas y saisir d’échanges confidentiels avec un avocat. L’équipe peut renseigner l’accompagnement et les coordonnées remises ; ces informations sont visibles par le patient. Le suivi ne peut pas accepter ou refuser un recours en justice et ne modifie pas le programme de soins.

**Il s’agit d’une demande interne, pas d’une saisine du tribunal.** Aucun avocat ou expert n’est contacté automatiquement ; aucune expertise judiciaire n’est garantie ou organisée par le logiciel. Il n’y a pas encore de compte avocat, de dossier de procédure judiciaire ou de calcul des délais de recours. Les informations officielles sont accessibles depuis l’écran :

- [L. 3211-3 — conseil d’un médecin ou avocat de son choix](https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI000028016782)
- [L. 3211-12-2 — assistance/représentation par un avocat à l’audience](https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI000048447117)
- [Service Public — soins pour troubles psychiatriques](https://www.service-public.gouv.fr/particuliers/vosdroits/F761)

## Accès depuis un téléphone et API distante

Le code est prêt à utiliser un backend distinct via `RUST_API_URL`. **Les adresses 127.0.0.1 utilisées ici ne sont pas accessibles depuis un autre appareil.** Pour un téléphone distant, il faudra héberger l’interface Next.js à une adresse HTTPS accessible et connecter son serveur à l’API Rust. Aucun déploiement distant ou accès public n’est activé dans cette livraison.

Configuration à définir lors d’un futur déploiement :

| Composant | Variable | Rôle |
| --- | --- | --- |
| Next.js | `RUST_API_URL` | URL du backend, côté serveur uniquement. |
| Next.js | `APP_ORIGIN` | Origine HTTPS exacte de l’interface, utilisée pour contrôler les mutations. |
| Rust | `PARCOURS_FRONTEND_ORIGIN` | Origine exacte autorisée de l’interface. Aucun joker. |
| Rust | `PARCOURS_API_HOST` | En-tête Host attendu par l’API ou son reverse proxy. |
| Rust | `PARCOURS_SECURE_COOKIES=true` | Cookies de session réservés à HTTPS. |
| Rust | `PARCOURS_BIND` | Écoute explicite du backend ; par défaut `127.0.0.1:8080`. |
| Rust | `PARCOURS_DB` | Chemin de la base SQLite sur le serveur. |

Un reverse proxy HTTPS et une politique d’hébergement et de protection adaptées aux données de santé restent à mettre en place avant un usage réel. Les données ne sont pas mises en cache hors ligne sur le mobile.

## Stockage et sécurité actuelle

La base de travail est `data/parcours.sqlite`. Les comptes et les données métier restent dans le même fichier SQLite. Les écritures sont transactionnelles. Les anciennes données métier sont conservées à l’ajout des comptes et des nouveaux modules.

- Mots de passe hachés avec Argon2id et sel aléatoire.
- Codes d’activation à usage unique et jetons de session stockés uniquement sous forme d’empreintes SHA-256.
- Cookies `HttpOnly`, `SameSite=Strict`, session de huit heures ; déconnexion et changement de mot de passe révoquent les sessions concernées.
- Temporisation après cinq échecs de connexion et limite globale des tentatives.
- Autorisation par rôle/dossier et filtrage des réponses, y compris après une mutation.
- Rejet des origines et hôtes inattendus, relais Next.js limité à l’API configurée, absence de CORS permissif.
- Permissions du fichier SQLite limitées au propriétaire sur macOS/Linux.

La base n’est pas chiffrée. Il n’y a pas encore de MFA, de signature professionnelle, de sauvegarde automatisée, de journal inviolable, d’audit de sécurité externe, d’intégration CPS/Pro Santé Connect ou de validation de conformité clinique. Les comptes locaux ne remplacent pas ces éléments. Utiliser des données fictives.

Pour une sauvegarde restaurable, arrêter les services puis copier le dossier `data` complet dans un emplacement protégé. La sauvegarde inclut les comptes, les sessions et leurs empreintes : elle doit rester confidentielle. L’export JSON des données métier ne réimporte pas les comptes et n’est pas une sauvegarde restaurable complète.

## Vérification et sources

```sh
cargo test --locked --offline
cd frontend
npm run build
```

Les tests Rust couvrent les règles métier, le versionnement, les transactions, les droits entre rôles/dossiers, l’identité de l’auteur, l’activation à usage unique, les sessions révoquées ou expirées et le volet contestation. Les vérifications d’interface utilisent une base de test séparée, jamais des mots de passe par défaut dans la base de travail.

- `frontend/app/` et `frontend/components/` : interface Next.js/React et relais API.
- `src/main.rs` : API et stockage.
- `src/auth.rs` : comptes, sessions et autorisations obligatoires.
- `src/medication.rs` : circuit des demandes et préparation.
- `src/nursing.rs` : soins à domicile.
- `src/rights.rs` : accompagnement de la contestation.
- `static/` : interface historique protégée, conservée pour compatibilité.
- `scripts/start.mjs` : lancement local multiplateforme.

## Curatelle et programme de soins

Les deux situations sont enregistrées séparément. Dans **Patients**, le médecin ou l’équipe affectée au dossier peut renseigner une curatelle simple, renforcée ou aménagée : dates figurant dans la décision, référence, état, périmètre d’assistance et coordonnées du curateur. Chaque modification ajoute une version et conserve les précédentes. Une date de fin passée est signalée pour vérification ; elle ne provoque aucun changement juridique automatique.

Le patient consulte les informations dans **Ma protection juridique**. Dans ses demandes d’accompagnement avocat/avis psychiatrique indépendant, il peut demander l’accompagnement de son curateur lorsque la mesure est renseignée en cours. Cette case est facultative et n’envoie rien au curateur. La demande conserve la référence et la version de la mesure connue au moment de l’enregistrement.

Enregistrer une curatelle ne crée aucun compte curateur, n’attribue aucun accès au dossier médical, ne permet pas de signer ou de décider à la place du patient et ne modifie pas le programme de soins. Les demandes du patient restent disponibles sans approbation du curateur dans l’application. Il n’y a pas encore de délégation d’accès nominative ni de gestion des mandats ; un éventuel accès curateur devra faire l’objet d’un développement distinct et d’autorisations documentées.

L’API `PUT /api/patients/{id}/protection` est réservée au médecin et à l’équipe affectés au dossier. Les informations de curatelle sont renvoyées au patient concerné et à ses professionnels affectés. Elles ne sont pas incluses dans les réponses destinées aux pharmacies et aux infirmiers à domicile.

Les règles affichées renvoient à [Service Public — santé sous tutelle ou curatelle](https://www.service-public.gouv.fr/particuliers/vosdroits/F33899) et [curatelle d’une personne majeure](https://www.service-public.gouv.fr/particuliers/vosdroits/F2094). Le type de curatelle ne suffit pas à déterminer le périmètre d’intervention : l’application conserve les éléments saisis d’après la décision, sans les déduire automatiquement.

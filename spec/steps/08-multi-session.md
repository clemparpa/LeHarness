# Étape 8 — Orchestrateur multi-sessions (local-first)

## Objectif
Un serveur local qui traite **plusieurs sessions en même temps** (goal acté,
prioritaire) : **un processus par session**, lancé via le lanceur de l'étape 3,
sans aucun runtime async. Le mode GitHub App remote (webhooks, jobs éphémères)
reste en v2 par-dessus, toujours dépriorisé. Mécanisme et bornes justifiés dans
[spec/ecosystem.md](../ecosystem.md) §8.

## Contrat / API
- `crates/harness/src/serve.rs` : `le-harness serve --repos-dir <dir> [--max-sessions N] [--session-timeout <secondes>]`
  (défaut `--max-sessions 8`).
  - Chaque session = un sous-processus `le-harness run --repo <chemin> ...`
    (protocole ligne-JSON de l'étape 3), avec son propre `SecretStore` et son
    propre **phantom token** : isolation par processus — un crash ou un hang
    d'une session ne touche pas les autres.
  - Un thread lecteur par session → `std::sync::mpsc::channel` côté
    orchestrateur : multiplexage sans runtime async.
  - **Bornes (fail-closed)** : `--max-sessions` atteint = refus ; **flock par
    repo** (deux sessions sur le même repo = refus) ; rlimits sur chaque fils
    via le lanceur ; timeout par message (`--session-timeout`, défaut 30 s) +
    kill propre du fils.
  - L'orchestrateur **ne détient aucun secret de session** : il route des lignes
    JSON déjà masquées ; log d'audit append-only par session (id, événements,
    jamais de contenu de secret).
  - CLI d'admin : lister les sessions actives, tuer une session (kill propre),
    afficher les bornes.
- Protocole orchestrateur ↔ session : ligne JSON par ligne ; une requête crée une
  session ou vise une session existante (taggée par id) ; réponses taggées par
  id ; réponse mal formée = erreur fail-closed + kill propre de la session.
- Aucun `unsafe` ; le cœur (étapes 1–4) est inchangé.

## Testable
- Tests unitaires `serve.rs` : parsing des requêtes, bornes (max-sessions atteint
  = refus), table session → processus.
- Tests e2e : N sessions simultanées contre `dummy-tool` (concurrence mesurée
  au wall-clock) ; flock (deuxième session sur le même repo = refus) ; kill
  propre en fin de vie (proptest : aucun processus fils ne reste vivant) ;
  timeout d'une session qui ne répond pas = kill + erreur.
- Gate PR vert : `cargo fmt --check`, `clippy -D warnings`, `cargo test`,
  `cargo deny` + `cargo audit`.

## Hors scope
- GitHub App remote (webhooks, jobs éphémères) : v2 par-dessus, toujours
  dépriorisé.
- Persistance transverse des sessions (étape 5, par session), ACP (étape 6),
  UI web.
- Runtime async (`tokio` & co) : non — voir `spec/ecosystem.md` §8.

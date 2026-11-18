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
  - **Bornes (fail-closed)** : `--max-sessions` atteint = refus ; rlimits sur
    chaque fils via le lanceur ; timeout par message (`--session-timeout`,
    défaut 30 s) + kill propre du fils.
  - **Pas de verrou de worktree (décidé nov. 2026, supprimé)** : plusieurs
    sessions peuvent travailler **en même temps sur le même worktree**, et a
    fortiori sur plusieurs worktrees du même repo. Conséquence documentée : en
    cas d'écritures concurrentes (`>`/`>>`, `sub -i`, `trusted` mutants) sur les
    mêmes fichiers, le résultat est une course d'écriture — **c'est la
    responsabilité de l'utilisateur**, pas un refus du harness. La politique
    (`trusted.txt` / `sensitives.txt`) est lue dans chaque worktree — chaque
    worktree a son propre checkout, donc sa propre politique.
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
  au wall-clock, y compris deux sessions sur le même worktree et sur deux
  worktrees du même repo — aucun verrou n'intervient) ; kill
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

## Implémentation (étape 8)

- `crates/harness/src/serve.rs` : `Orchestrator` + boucle `serve()` (un thread
  lecteur stdin, un thread lecteur par session → `mpsc` unique, boucle de
  multiplexage simple, aucun runtime async). Chaque session est un
  sous-processus `le-harness run --repo <repos-dir>/<repo> --json
  --secrets-file <state>/serve-secrets/<id>.env --state-dir <state>/sessions
  [--tool <binaire>]`, spawn via `process::spawn_piped` (brique extraite du
  lanceur étape 3, cœur inchangé : `Tool` l'encapsule toujours pour le mode
  synchrone). Le fichier secrets par session ne contient que le **phantom
  token** généré par l'orchestrateur — l'orchestrateur lui-même a un
  `SecretStore` vide et ne route que des lignes déjà masquées par le fils.
- `run --json` (mode additif, comportement texte inchangé) : une ligne
  `ToolRequest` en entrée → une ligne `ToolResponse` en sortie ;
  `status` / `checkpoint` / `quit` sont des commandes internes, le reste est
  forwardé à l'outil.
- Protocole orchestrateur (ligne JSON sur stdin/stdout de `serve`), parsing
  fail-closed (`parse_request`, champs inconnus rejetés) :
  - `{"op":"start","repo":"r1"[,"id":"sess-x"]}` → `started` / `error`
  - `{"op":"call","session":"...","input":"..."[,"id":N]}` → `accepted`
    immédiat, puis `result` (taggé session + req) poussé quand le fils répond
  - `{"op":"kill","session":"..."}` → `killed` (kill propre + reap)
  - `{"op":"list"}` → `sessions` ; `{"op":"bounds"}` → bornes + actif ;
    `{"op":"quit"}` → `ended` puis toutes les sessions sont tuées proprement
  - réponse de session mal formée = `error` fail-closed + kill propre de la
    session ; timeout par message (`--session-timeout`) = `error` + kill
  - audit append-only par session : `<state>/audit/<id>.jsonl`, événements
    (start/accepted/result/timeout/malformed/killed/ended) sans jamais de
    contenu de requête/réponse
- Bornes effectives : `--max-sessions` (refus fail-closed), timeout par message
  + kill propre, ligne bornée à 1 MiO (lanceur). **rlimits sur les fils** :
  pas faisables en `std` pur sans `unsafe` ni dépendance (`setrlimit` exige
  `libc`) → restent côté **déploiement v2**, comme décidé à l'étape 3.
- Pas de verrou de worktree : plusieurs sessions sur le même worktree ou
  plusieurs worktrees du même repo sont autorisées (testé en e2e).

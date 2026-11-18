# Étape 5 — Checkpoints locaux + reprise

## Objectif
Brancher une vraie persistance disque derrière le trait `Persistence` de
l'étape 1 (fichiers JSON v1, un par session, écrits de façon atomique), et
permettre de reprendre une session depuis la CLI (`run --resume <id>`), de
part et d'autre d'un arrêt du processus.

## Contrat / API
- `crates/harness/src/session.rs` : `SessionId`, `SessionState` et `Snapshot`
  dérivent en plus `serde::Serialize` / `serde::Deserialize` (déjà dans les
  dépendances, aucun ajout). `SessionId` implémente `std::str::FromStr`
  (id vide = erreur `InvalidSessionId`), pour le parsing du flag `--resume`.
- `crates/harness/src/persistence.rs` (nouveau module, exporté dans `lib.rs`) :
  `FilePersistence { dir: PathBuf }` implémente `Persistence`.
  - `FilePersistence::new(dir) -> Result<Self, PersistenceError>` : crée le
    répertoire (récursif) si absent ; échec = `PersistenceError::Backend`.
  - `FilePersistence::default_dir() -> Result<PathBuf, PersistenceError>` :
    `$HOME/.local/share/le-harness/sessions` ; **fail-closed** si `HOME` est
    absent (message demandant `--state-dir`).
  - Un fichier par session : `<dir>/<id>.json`. L'id est validé avant toute
    construction de chemin : uniquement `[A-Za-z0-9._-]`, sinon
    `PersistenceError::Backend` (fail-closed contre `../…` via `--resume`).
  - `save` = sérialisation JSON du `Snapshot`, écriture dans `<id>.json.tmp`
    puis `sync_all` puis `rename` atomique vers `<id>.json` (upsert : le
    dernier snapshot d'une session écrase le précédent ; un crash pendant
    l'écriture ne corrompt jamais le fichier en place).
  - `load` : fichier absent = `Ok(None)` ; JSON illisible/corrompu = `Err`
    (jamais ignoré silencieusement) ; autre erreur I/O = `Err`.
  - Synchrone, aucun `unsafe`, aucune nouvelle dépendance (`serde_json` +
    `std::fs`). Pas de `redb` en v1 (volume non justifié).
- `crates/harness/src/run.rs` : `RunConfig` gagne deux champs :
  `state_dir: Option<PathBuf>` et `resume: Option<SessionId>`
  (`RunConfig::new` les met à `None`).
  - `run()` construit le store : `config.state_dir` sinon
    `FilePersistence::default_dir()?` (`RunError::Persistence` en cas d'échec).
  - Si `config.resume` est fourni : `Session::resume(id, repo, NoHooks, store)`
    et la ligne d'ouverture devient `session <id> resumed on <repo> (<n>
    checkpoints)` ; sinon `Session::start(...)` comme avant (`session <id>
    started on …`). Les refus fail-closed du cœur (id inconnu, session
    terminée, racine différente) remontent en `RunError::Session` → exit 2.
  - Le reste de la boucle est inchangé (`checkpoint` persiste désormais sur
    disque à chaque appel). La persistance en mémoire de l'étape 4 est
    supprimée.
- `crates/harness/src/main.rs` : la sous-commande `run` gagne
  `--resume <SESSION_ID>` (parsé via `FromStr`) et `--state-dir <PATH>`.
  Toute erreur = exit 2, message sur stderr (inchangé).
- Aucun `unsafe` ; synchrone de bout en bout.

## Testable
- Tests unitaires `persistence.rs` (avec `tempfile`) : save→load roundtrip ;
  `load` d'un id inconnu = `None` ; double save = upsert (dernier snapshot
  gagnant) ; fichier corrompu = `Err` ; id invalide (`../evil`, vide, `/`)
  = `Err` sur save et load ; `new` crée un répertoire imbriqué ; après save,
  aucun fichier `.tmp` ne traîne ; le contenu est du JSON valide avec les
  champs attendus ; `default_dir` = `$HOME/.local/share/le-harness/sessions`.
- Tests unitaires `run.rs` : stop/reprise — `run` avec `checkpoint` x2 puis
  `quit`, extraction de l'id dans la sortie, second `run` avec
  `resume = Some(id)` → `resumed`, `checkpoint 3`, `RunOutcome.checkpoints
  == 3` ; resume d'un id inconnu = `NotFound` ; resume d'une session terminée
  = `AlreadyEnded` ; resume avec un autre root = `RootMismatch`.
- Tests e2e (`crates/harness/tests/e2e.rs`, vrai binaire via
  `CARGO_BIN_EXE_le-harness`) : session scriptée `checkpoint\ncheckpoint\nquit`
  avec `--state-dir` dans un `tempfile`, extraction de l'id sur stdout, puis
  second processus `run --resume <id>` → exit 0, `resumed` + `checkpoint 3` ;
  `--resume` d'un id inconnu → exit 2 ; `--resume` d'une session terminée →
  exit 2.

## Hors scope
`redb` ou tout autre backend embarqué (volume non justifié en v1), rétention /
GC des vieux fichiers de session, chiffrement au repos des snapshots,
snapshot enrichi (historique de conversation — le `Snapshot` reste celui de
l'étape 1), orchestrateur multi-sessions (étape 8), transport ACP (étape 6).

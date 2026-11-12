# Étape 3 — Lanceur générique de sous-processus

## Objectif
Un lanceur maison (`crates/harness/src/process.rs`) qui exécute un outil externe
en sous-processus avec un protocole **une ligne JSON par ligne** sur
stdin/stdout (générique — aucun lien avec la-console), un timeout avec kill
propre du fils, et le rejet fail-closed de toute réponse mal formée.

## Contrat / API
- Protocole : le harness envoie une ligne JSON `{"id": <u64>, "input": <string>}`
  (une seule ligne, sans `\n` embarqué) ; l'outil répond par une ligne JSON
  `{"ok": bool, "code": i32, "stdout": string, "stderr": string, "truncated": bool}`.
  `truncated` est optionnel côté réception (`#[serde(default)]`).
- `ToolRequest { id: u64, input: String }` (`serde::Serialize`) ;
  `ToolResponse { ok, code, stdout, stderr, truncated }` (`serde::Deserialize`,
  `deny_unknown_fields` — toute réponse avec un champ inconnu est mal formée).
- `parse_line(line: &str) -> Result<ToolResponse, ProcessError>` : parse
  **fail-closed** d'une ligne de réponse — JSON invalide, champ requis absent
  ou de mauvais type, champ inconnu, ligne vide = erreur `Malformed`, jamais
  d'interprétation partielle. Fonction publique (cible de fuzz `parse_line`).
- `Tool::spawn(binary, args, secrets, env_allowlist) -> Result<Tool, ProcessError>` :
  spawn du binaire avec stdin/stdout pipés ; **l'env du fils est construit
  uniquement à partir d'une allowlist explicite de noms** (ex. `PATH`, `HOME`,
  `LANG`, `TERM`, `TMPDIR`) recopiés depuis l'env courant — jamais d'héritage
  complet. Avant le spawn, vérification fail-closed : **aucune valeur de secret
  connue ne doit apparaître dans le binaire, les args, les valeurs d'env
  transmises** (sinon `ProcessError::SecretLeak`, le fils n'est pas lancé).
- `Tool::call(&mut self, req, timeout) -> Result<ToolResponse, ProcessError>` :
  sérialise la requête en une ligne, l'écrit sur le stdin du fils, puis lit la
  ligne de réponse avec `recv_timeout` (thread lecteur dédié + channel) :
  - timeout dépassé = `ProcessError::Timeout`, le fils est tué (`kill` + `wait`)
    et plus aucun processus fils ne reste vivant ;
  - réponse mal formée = `ProcessError::Malformed`, idem (l'outil est tué) ;
  - EOF du fils avant toute réponse = `ProcessError::Closed` ;
  - la ligne lue est bornée (`MAX_LINE_BYTES` = 1 MiO) côté thread lecteur ;
  - avant de renvoyer la réponse, `stdout` et `stderr` passent par
    `SecretStore::mask` (toute occurrence d'un secret connu devient `***`) ;
  - une requête dont la ligne sérialisée contient un secret est refusée
    (`SecretLeak`) avant écriture.
- `impl Drop for Tool` : tue et ré reap (`kill` + `wait`) le fils si encore
  vivant — aucun zombie après la fin de vie du `Tool`.
- Erreurs typées `ProcessError` via `thiserror` (`Spawn`, `Io`, `Timeout`,
  `Malformed`, `LineTooLong`, `SecretLeak`, `Closed`).
- Aucun `unsafe` ; `std::process` uniquement (pas de crate de process).

## Testable
- Tests unitaires dans `process.rs` + proptest :
  - `parse_line` : réponse valide (avec/sans `truncated`), JSON invalide, champ
    absent, mauvais type, champ inconnu, ligne vide, ligne trop longue ;
  - proptest « toute ligne reçue d'un outil : une réponse JSON invalide est
    rejetée fail-closed » (round-trip serialize → parse, et toute entrée
    arbitraire est soit rejetée, soit une réponse complète) ;
  - proptest « après timeout ou kill, aucun processus fils ne reste vivant ».
- Tests e2e (`crates/harness/tests/e2e.rs`) contre un **binaire bidon committé**
  (`src/bin/dummy-tool.rs`, résolu via `env!("CARGO_BIN_EXE_dummy-tool")`) :
  requête/réponse nominale, réponse mal formée = `Malformed` + fils tué, timeout
  sur commande longue = `Timeout` + fils tué, `printenv` du fils ne contient
  aucun secret, spawn avec un secret dans les args = `SecretLeak` sans spawn,
  masquage d'un secret présent dans la sortie de l'outil.
- **Fuzz** (`fuzz/`, workspace cargo-fuzz exclu du workspace principal) :
  cibles `parse_line` et `mask_secrets`, **corpus seedé committé** dans
  `fuzz/corpus/<cible>/` (cas réalistes : réponses valides, champs manquants,
  JSON invalide, unicode, lignes longues pour `parse_line` ; secret + texte
  arbitraire pour `mask_secrets`). Nightly : fuzz 30 min
  (`-max_total_time=1800 -max_len=4096`, cible `x86_64-unknown-linux-gnu`) +
  `cargo mutants` sur `secrets.rs` + `process.rs` (score ≥ 80 %).

## Hors scope
Brancher la-console comme outil réel (étape 7), env des fils par fd/socket
unix pour les credentials (mécanisme la-console), limites rlimit du fils
(côté déploiement v2), orchestrateur GitHub App (étape 8).

# Étape 0 — Squelette + arsenal

## Objectif
Un workspace Cargo conforme à `spec/structure.md` qui compile et merge avec CI verte.

## Contrat / API
- `Cargo.toml` virtuel (workspace), `crates/harness` membre ; `fuzz/` exclu.
- `crates/harness/src/lib.rs` avec `#![forbid(unsafe_code)]`.
- `crates/harness/src/main.rs` : CLI clap minimale (ex. `--help` fonctionnel).
- `rustfmt.toml`, `deny.toml`, `.gitignore` (incl. `/target/`, `mutants.out/`, artifacts fuzz).
- CI `.github/workflows/ci.yml` : `fmt --check`, `clippy -D warnings`, `cargo test`
  (matrice ubuntu + macOS), `cargo deny check` + `cargo audit`, garde-fou anti-fork
  sur tous les jobs.

## Testable
Un test bidon dans `crates/harness` ; la PR merge avec CI verte sur toutes les jobs.

## Hors scope
Tout code métier (session, secrets, process). Le nightly (fuzz + mutants) arrive
avec l'étape 3, quand les cibles de fuzz existent.

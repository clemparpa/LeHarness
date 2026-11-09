# Stratégie de test et outillage

Reprise de la stratégie validée sur la-console (`clemparpa/LaConsole`), adaptée
au harness. Le harness est pilotable sans infrastructure (sessions locales,
binaires de test) : tout se teste en `cargo test`. TDD systématique.

## Outillage local (boucle de dev)

| Check | Commande | Temps estimé |
|---|---|---|
| Format | `cargo fmt --all` (check : `-- --check`) | < 1 s |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | 5–20 s incrémental |
| Tests unitaires | `cargo test -p harness --lib` | 2–10 s |
| Tests d'intégration / e2e | `cargo test -p harness --test '*'` | 5–30 s |

Gate « prêt pour PR » : fmt + clippy + `cargo test` complet ≈ 30 s à 1 min en
incrémental. `#![forbid(unsafe_code)]` dans `lib.rs` pour traduire « aucun
`unsafe` en v1 » en contrainte compilateur.

## Niveaux de test

- **Tests unitaires** (`#[test]` + `cargo test`), `tempfile` en dev-dependency.
- **Property-based (`proptest`)** sur les invariants de sécurité :
  - « pour toute séquence d'octets, le masqueur de secrets remplace toute
    occurrence d'un secret connu dans la sortie » ;
  - « pour tout binaire lancé, après timeout ou kill aucun processus fils ne
    reste vivant » ;
  - « pour toute ligne reçue d'un outil, une réponse JSON invalide est rejetée
    fail-closed (jamais interprétée partiellement) ».
- **Tests e2e** : spawn du vrai binaire (`env!("CARGO_BIN_EXE_le-harness")`),
  scénarios de session de bout en bout (démarrage, commande via outil,
  checkpoint, fin, reprise) avec un binaire outil bidon committé.
- **Fuzzing (`cargo-fuzz`, libFuzzer coverage-guided)** sur les cibles :
  - `parse_line` (parseur de lignes JSON du protocole outil) ;
  - `mask_secrets` (masqueur de secrets).
  Corpus seedé avec des cas réalistes. Les crashers trouvés sont committés
  comme tests de régression.
- **Mutation testing (`cargo mutants`)** sur `secrets.rs` + `process.rs` :
  un mutant qui survit = un trou de couverture, potentiellement une fuite de
  secret ou un processus zombie. `mutants.out/` est dans le `.gitignore`.
- **Couverture (`cargo llvm-cov`)** avec seuil sur les modules sécurité
  (`secrets.rs`, `process.rs`).
- **Licences / advisories (`cargo deny` + `cargo audit`)** et `cargo shear`
  (dépendances inutilisées) pour garder la surface minimale et auditable.
- **Vérification formelle (Kani/Prusti)** : envisagée en v2 ; pas en v1.

## CI (GitHub Actions)

Deux niveaux, selon le coût :

- **Gate à chaque PR** (runners GitHub-hosted, rapides) : `cargo fmt --check`,
  `clippy -D warnings`, `cargo test` complet (matrice ubuntu + macOS),
  `llvm-cov` avec seuil, `deny`/`audit`, **garde-fou anti-fork** sur tous les
  jobs (`if: github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository`)
  — jamais de jobs coûteux exposés aux PR de forks sur un dépôt public.
- **Nightly** (cron + `workflow_dispatch`, aucun garde-fork nécessaire) :
  fuzz long (30 min) sur `parse_line` + `mask_secrets`, `cargo mutants` avec
  seuil de score ≥ 80 % vérifié via `mutants.out/outcomes.json`.

## Fuzz

Workspace `fuzz/` exclu du workspace principal (`exclude = ["fuzz"]`) :
`cargo-fuzz` sur toolchain nightly, cible `x86_64-unknown-linux-gnu` (ASan
incompatible avec musl statique), `-max_total_time=1800 -max_len=4096`, corpus
seedé committé dans `fuzz/corpus/<cible>/`, artifacts non committés.

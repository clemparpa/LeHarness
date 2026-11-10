# Feuille de route — LeHarness

## Principes

- Chaque étape = une PR indépendante, **testable et avec CI verte avant merge** (détail dans `spec/steps/NN-*.md`).
- TDD systématique (voir `spec/testing.md`).
- Cœur sécurité (`SecretStore`, lanceur de sous-processus, hooks de session) écrit maison, volontairement.
- Aucun `unsafe` en v1 (`#![forbid(unsafe_code)]`).
- **Le harness est 100 % indépendant de la-console** : deux repos séparés (`clemparpa/LaConsole`), aucune dépendance de code. La console n'est qu'un outil optionnel, branché comme binaire externe (étape 7).
- **Local-first** : le harness tourne en local sur un repo local ; le mode GitHub App remote (étape 8) s'ajoute par-dessus sans toucher au cœur.
- Sur le dépôt public : workflows de PR de forks bloqués (garde-fou `if:` dans la CI) — jamais de jobs coûteux exposés aux forks.

## Étape 0 — Squelette + arsenal (fondation) ✅

- Workspace conforme à `spec/structure.md` : `Cargo.toml` virtuel, `crates/harness` (binaire + lib), `lib.rs` avec `#![forbid(unsafe_code)]`, `main.rs` avec CLI clap minimale.
- Fichiers de config : `rustfmt.toml`, `deny.toml`, `.gitignore` (incl. `mutants.out/`, artifacts fuzz).
- CI GitHub Actions (gate PR) : `cargo fmt --check`, `clippy -D warnings`, `cargo test`, matrice ubuntu + macOS, `cargo deny` + `cargo audit`, garde-fou anti-fork sur les jobs.
- **Testable** : une PR qui merge avec CI verte sur un code minimal (un test bidon suffit).

## Étape 1 — Cœur : session + hooks 🚧 (cette PR)

- Spec : [spec/steps/01-session-hooks.md](spec/steps/01-session-hooks.md)
- `session.rs` : `Session` (id, root du repo, état) + cycle de vie avec hooks `on_start` / `on_checkpoint` / `on_end` / `on_resume`.
- La persistance est un **trait injecté** par la couche au-dessus : le cœur n'écrit jamais sur disque lui-même.
- **Testable** : tests unitaires du cycle de vie avec un double de test (persistance en mémoire).

## Étape 2 — Gestion des secrets (maison) 🚧 (cette PR)

- Spec : [spec/steps/02-secrets.md](spec/steps/02-secrets.md)

- `secrets.rs` : `SecretStore` en mémoire uniquement. Chargement au lancement (variable d'env ou fichier de config local du user), refresh automatique (ex. token GitHub App, validité 1 h).
- Règles absolues : jamais écrite sur disque, jamais dans l'env ou l'argv des sous-processus lancés, masquage `***` de toute occurrence dans toute sortie transmise à l'extérieur.
- **Testable** : tests unitaires + proptest « un secret ne fuute jamais dans une sortie masquée ».

## Étape 3 — Lanceur générique de sous-processus

- `process.rs` : spawn d'un binaire externe avec protocole **ligne JSON par ligne** (générique — aucun lien avec la-console), timeout, kill propre du fils, rejet fail-closed des réponses mal formées.
- Cibles de fuzz : parseur de lignes JSON + masqueur de secrets. Nightly : fuzz 30 min + `cargo mutants` (seuil ≥ 80 %) sur `secrets.rs` + `process.rs`.
- **Testable** : e2e contre un binaire bidon de test committé (`CARGO_BIN_EXE_`-style).

## Étape 4 — CLI local

- `le-harness run --repo <chemin>` : démarre une session sur un repo local, lit la politique du repo (`trusted.txt`, `sensitives.txt`), boucle interactive locale (stdin → session → outil).
- **Testable** : e2e scripté.

## Étape 5 — Checkpoints locaux + reprise

- Persistance de session en local via le trait de l'étape 1 : fichiers JSON (v1) ou `redb` si le volume le justifie.
- **Testable** : test stop/reprise de session.

## Étape 6 — ACP en couche de transport optionnelle (entrée)

- Implémenter le rôle `Agent` du SDK officiel `agent-client-protocol` par-dessus le cœur : transport JSON-RPC uniquement, zéro logique métier. Le harness devient pilotable depuis un éditeur compatible ACP (ex. Zed).
- **Testable** : avec l'exemple client du SDK officiel.

## Étape 7 — Connecteur la-console (outil optionnel)

- Brancher `la-console` (repo séparé `clemparpa/LaConsole`) comme outil via le lanceur générique de l'étape 3. Dépendance par **binaire**, jamais par code.
- **Testable** : e2e contre le vrai binaire `la-console`.

## Étape 8 — Orchestrateur GitHub App (remote)

- Webhooks, jobs éphémères, persistance et verrou anti-parallèle gérés par cette couche au-dessus du harness. Le cœur est inchangé.
- **Testable** : tests d'intégration avec un serveur de test local.

## Hors scope v1

- User namespace + chroot côté déploiement serveur (defense in depth, v2 — voir `spec/security.md`).
- Conteneurs / navigateur headless (voir `docs/ideas/backlog.md`).

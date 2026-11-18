# Feuille de route — LeHarness

## Principes

- Chaque étape = une PR indépendante, **testable et avec CI verte avant merge** (détail dans `spec/steps/NN-*.md`).
- TDD systématique (voir `spec/testing.md`).
- Cœur sécurité (`SecretStore`, lanceur de sous-processus, hooks de session) écrit maison, volontairement.
- Aucun `unsafe` en v1 (`#![forbid(unsafe_code)]`).
- **Le harness est 100 % indépendant de la-console** : deux repos séparés (`clemparpa/LaConsole`), aucune dépendance de code. La console n'est qu'un outil optionnel, branché comme binaire externe (étape 7).
- **Local-first** : le harness tourne en local sur un repo local. L'orchestration multi-sessions (étape 8) est local-first ; le mode GitHub App remote reste en v2, dépriorisé (voir étape 8).
- **Concurrence bornée, sans runtime async** : multi-sessions = un processus par session (étape 8), appels d'outils parallèles = threads bornées `--max-parallel-tools` (étape 9). Jamais de runtime async (`tokio` & co) en dépendance directe — arguments et bornes dans [spec/ecosystem.md](spec/ecosystem.md).
- **Le harness est le broker de credentials des outils** : il joue le rôle de **reverse proxy local** (pattern « phantom token », inspiré de [nono](https://nono.sh)) — les outils ne reçoivent qu'un token fantôme de session ; le vrai secret, détenu uniquement par le harness, est injecté à la sortie, uniquement vers les destinations autorisées.
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

## Étape 3 — Lanceur générique de sous-processus 🚧 (cette PR)

- Spec : [spec/steps/03-process.md](spec/steps/03-process.md)
- `process.rs` : spawn d'un binaire externe avec protocole **ligne JSON par ligne** (générique — aucun lien avec la-console), timeout, kill propre du fils, rejet fail-closed des réponses mal formées.
- Cibles de fuzz : parseur de lignes JSON + masqueur de secrets. Nightly : fuzz 30 min + `cargo mutants` (seuil ≥ 80 %) sur `secrets.rs` + `process.rs`.
- **Testable** : e2e contre un binaire bidon de test committé (`CARGO_BIN_EXE_`-style).

## Étape 4 — CLI local 🚧 (cette PR)

- Spec : [spec/steps/04-cli-local.md](spec/steps/04-cli-local.md)
- `le-harness run --repo <chemin>` : démarre une session sur un repo local, lit la politique du repo (`trusted.txt`, `sensitives.txt`), boucle interactive locale (stdin → session → outil).
- **Testable** : e2e scripté.

## Étape 5 — Checkpoints locaux + reprise

- Persistance de session en local via le trait de l'étape 1 : fichiers JSON (v1) ou `redb` si le volume le justifie.
- **Testable** : test stop/reprise de session.

## Étape 6 — Proxy ACP en couche de transport optionnelle (entrée)

- Le harness joue le rôle **`Proxy`** ACP (pas `Agent`) : placé entre l'éditeur
  (client ACP, ex. Zed) et l'agent LLM externe, il intercepte les appels
  d'outils, les exécute via la-console de façon confinée (politique du repo,
  masquage des secrets, phantom token) et renvoie les résultats masqués.
  La boucle LLM et le chat restent côté agent externe — le harness ne parle
  jamais au modèle.
- Transport JSON-RPC uniquement, zéro logique métier dans cette couche, cœur
  inchangé. v1 : rôle `Proxy` maison synchrone au-dessus du protocole
  ligne-JSON ; SDK officiel `agent-client-protocol` (async, `conductor`/Proxy)
  si besoin mesuré.
- **Testable** : avec l'exemple client du SDK officiel + un agent bidon.

## Étape 7 — Connecteur la-console (outil optionnel)

- Brancher `la-console` (repo séparé `clemparpa/LaConsole`) comme outil via le lanceur générique de l'étape 3. Dépendance par **binaire**, jamais par code.
- **Testable** : e2e contre le vrai binaire `la-console`.

## Étape 8 — Orchestrateur multi-sessions (local-first) 🔜 prioritaire

- Spec : [spec/steps/08-multi-session.md](spec/steps/08-multi-session.md)
- **Goal acté** : le serveur traite plusieurs sessions en même temps. Un **processus par session** via le lanceur de l'étape 3 (protocole ligne-JSON), un thread lecteur par session → `mpsc`. Zéro runtime async.
- Bornes fail-closed : `--max-sessions` global, **flock par worktree** (racine canonique du worktree passée à `--repo` : plusieurs worktrees du même repo en parallèle = OK, deux sessions sur le même worktree = refus), rlimits sur chaque fils, timeout par message + kill propre, log d'audit append-only par session. L'orchestrateur ne détient aucun secret de session (il route).
- Chaque session a son propre `SecretStore` + phantom token (broker) : isolation par processus.
- Le mode GitHub App remote (webhooks, jobs éphémères) reste **en v2 par-dessus**, toujours dépriorisé.
- **Testable** : tests d'intégration multi-sessions simultanées contre des outils bidons (concurrence mesurée, flock, bornes, kill propre).

## Étape 9 — Interface TUI Rust de base + appels d'outils en parallèle (bornés) 🔜 (prioritaire)

- Spec : [spec/steps/09-tui.md](spec/steps/09-tui.md)
- **Goal acté** : appels d'outils lancés en parallèle (résolution plus rapide de l'agent) : `submit` non bloquant + worker threads bornées (`--max-parallel-tools`, défaut 4, plafond 16), résultats taggés par id via `mpsc` ; les requêtes avec redirection (`>`/`>>`) sont sérialisées (mutex) ; chaque appel garde timeout + kill + masquage.
- Une couche de transport **TUI (terminal)** en Rust, au-dessus du cœur de session : le harness standard devient pilotable depuis une interface terminal de base.
- Stack retenue (tour d'horizon dans [spec/ecosystem.md](spec/ecosystem.md)) : **ratatui + crossterm** (boucle `poll`/`read` + thread d'événements → `mpsc`, rendu sur stderr), `color-eyre` + `signal-hook` pour la restauration propre du terminal.
- Couche optionnelle comme les autres : le cœur fonctionne sans.
- **Testable** : e2e scripté contre la TUI (`TestBackend` + scénario clavier/PTY) + test de concurrence au wall-clock sur les appels parallèles.

## Hors scope v1

- User namespace + chroot côté déploiement serveur (defense in depth, v2 — voir `spec/security.md`).
- Conteneurs / navigateur headless (voir `docs/ideas/backlog.md`).

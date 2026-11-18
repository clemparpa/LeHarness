# Étape 9 — Interface TUI Rust de base + appels d'outils en parallèle (bornés)

## Objectif
Deux goals actés prioritaires, livrés ensemble :
1. Une couche de transport **TUI (terminal plein écran)** au-dessus du cœur de
   session : `le-harness tui --repo <chemin>` pilote une session depuis une
   interface terminal standard (**ratatui + crossterm**), couche optionnelle —
   le cœur fonctionne sans.
2. Des **appels d'outils en parallèle, bornés**, dans le cœur : l'agent peut
   soumettre plusieurs appels sans attendre, pour résoudre plus vite.

Stack et décision de concurrence dans [spec/ecosystem.md](../ecosystem.md).

## Contrat / API
- `crates/harness/src/tui.rs` : la couche TUI, factorisée hors de `main`.
  - `ratatui` 0.30 (rendu immediate-mode) + `crossterm` 0.29 (backend par défaut :
    raw mode, alternate screen, événements clavier/souris/resize). Rendu sur
    **stderr** ; stdout reste réservé au protocole ligne-JSON vers l'extérieur.
  - Boucle sans runtime async : thread d'événements crossterm
    (`event::poll(tick)` + `event::read()`) → `std::sync::mpsc::channel` → boucle
    principale. **Le même canal multiplexe les résultats des appels d'outils
    parallèles** (un worker thread par appel) et un tick périodique.
  - `color-eyre` installé **avant** `ratatui::init()` / `ratatui::run()` (panic
    hook qui restaure le terminal) ; `signal-hook` pour SIGINT/SIGTERM →
    restauration propre puis `session.end()`.
  - Layout de base : transcript (événements de session via les hooks de l'étape 1
    + sorties d'outils **ré-masquées** par le `SecretStore`, taggées par id de
    requête), zone de saisie (`ratatui-textarea` ou widget input), barre de statut
    (id de session, état, spinner `throbber-widgets-tui` + nombre d'appels en
    cours).
  - CLI : `le-harness tui --repo <chemin> [--tool <binaire>] [--secrets-file <chemin>] [--tool-timeout <secondes>] [--max-parallel-tools N]`,
    mêmes règles fail-closed que `run` (exit 2 + stderr).
  - `Esc` / `Ctrl-C` = fin de session propre.
  - Persistance en mémoire (l'étape 5 apportera la persistance fichier).
  - Aucun `unsafe` ; cœur (`session.rs`, `secrets.rs`, `process.rs`, `policy.rs`)
    inchangé dans son modèle.
- **Appels d'outils en parallèle (bornés)**, dans le cœur (`run.rs` aussi, pas
  seulement la TUI) :
  - `submit(request) -> id` non bloquant ; worker thread par appel (sous-processus
    via le lanceur de l'étape 3, timeout + kill + masquage par appel) ; résultats
    taggés par id, poussés dans le `mpsc` au fur et à mesure.
  - Borne `--max-parallel-tools` (défaut 4, plafond 16) : au-delà, la soumission
    attend une place (fail-closed, jamais de fork illimité).
  - **Sérialisation des écritures** : toute requête contenant une redirection
    (`>` ou `>>` dans la ligne) passe par un mutex — jamais deux écritures
    concurrentes sur le même repo ; les appels read-only tournent en parallèle.
  - Quota disque total borné à N × quota par commande (les bornes par appel de
    l'outil s'appliquent, plus le plafond global).
- Dépendances ajoutées (figées dans `spec/structure.md`) : `ratatui`, `crossterm`,
  `color-eyre`, `signal-hook`, `ratatui-textarea` (ou `tui-input`),
  `throbber-widgets-tui`. La concurrence elle-même n'ajoute **aucune** dépendance
  (`std::thread` + `std::process`).

## Testable
- Tests unitaires `ratatui::backend::TestBackend` : rendu du layout, saisie →
  `submit` non bloquant, affichage d'une sortie avec secret masqué (`***`),
  `Esc` → fin de session, restauration du terminal au drop.
- Tests unitaires cœur (`run.rs`) : 2 soumissions vers un `dummy-tool` qui dort
  1 s terminent en < 1,5 s au wall-clock (concurrence réelle), résultats taggés
  par id ; `--max-parallel-tools 1` force la séquentialité (≈ 2 s) ; une requête
  avec `>` est sérialisée même avec de la parallélisme disponible.
- Tests e2e scriptés : spawn du vrai binaire (`env!("CARGO_BIN_EXE_le-harness")`)
  avec clavier simulé (PTY ou événements injectés) dans un `tempfile` avec
  `trusted.txt` + `sensitives.txt` : session complète (démarrage, appels outils
  parallèles via `dummy-tool`, checkpoint, quit), sorties masquées, exit 2 sans
  politique / repo inexistant.
- Gate PR vert : `cargo fmt --check`, `clippy -D warnings`, `cargo test` complet,
  `cargo deny` + `cargo audit` sur les nouvelles dépendances.

## Hors scope
- Persistance fichier des checkpoints (étape 5), transport ACP (étape 6),
  connecteur la-console réel (étape 7), orchestrateur multi-sessions (étape 8).
- Souris avancée, thèmes, images (`ratatui-image`), mode inline,
  `reedline` (autre mode), `tuirealm` (réévalué si l'UI grossit beaucoup).
- Runtime async / `event-stream` : non — concurrence par threads/processus bornés
  (`spec/ecosystem.md` §8).

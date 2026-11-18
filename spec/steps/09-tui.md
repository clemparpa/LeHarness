# Étape 9 — Interface TUI Rust de base

## Objectif
Une couche de transport **TUI (terminal plein écran)** au-dessus du cœur de
session : `le-harness tui --repo <chemin>` pilote une session depuis une interface
terminal standard (**ratatui + crossterm**, boucle 100 % synchrone), comme couche
optionnelle — le cœur fonctionne sans. Choix de stack actés dans
[spec/ecosystem.md](../ecosystem.md).

## Contrat / API
- `crates/harness/src/tui.rs` : la couche TUI, factorisée hors de `main` pour être
  testable.
  - `ratatui` 0.30 (rendu immediate-mode) + `crossterm` 0.29 (backend par défaut :
    raw mode, alternate screen, événements clavier/souris/resize). Rendu sur
    **stderr** ; stdout reste réservé au protocole ligne-JSON vers l'extérieur.
  - Boucle **synchrone** : thread d'événements crossterm (`event::poll(tick)` +
    `event::read()`) poussés dans un `std::sync::mpsc::channel`, consommés par la
    boucle principale. Aucun runtime async (critères de réouverture :
    `spec/ecosystem.md` §8).
  - `color-eyre` installé **avant** `ratatui::init()` / `ratatui::run()` (panic hook
    qui restaure le terminal) ; `signal-hook` pour SIGINT/SIGTERM → restauration
    propre du terminal puis `session.end()`.
  - Layout de base : zone de transcript (événements de session via les hooks de
    l'étape 1 + sorties d'outils **ré-masquées** par le `SecretStore`), zone de
    saisie (`ratatui-textarea` ou widget input), barre de statut (id de session,
    état, spinner `throbber-widgets-tui` pendant un appel outil).
  - Entrée validée → `ToolRequest { id, input }` envoyée à l'outil `--tool` via le
    lanceur de l'étape 3 (mêmes timeout/kill) ; la réponse est affichée après
    masquage. `Esc` / `Ctrl-C` = fin de session propre.
  - CLI : `le-harness tui --repo <chemin> [--tool <binaire>] [--secrets-file <chemin>] [--tool-timeout <secondes>]`,
    mêmes règles fail-closed que `run` (exit 2 + message sur stderr).
  - Persistance en mémoire (l'étape 5 apportera la persistance fichier).
  - Aucun `unsafe` ; le cœur (`session.rs`, `secrets.rs`, `process.rs`, `policy.rs`)
    est inchangé.
- Dépendances ajoutées (figées dans `spec/structure.md`) : `ratatui`, `crossterm`,
  `color-eyre`, `signal-hook`, `ratatui-textarea` (ou `tui-input`),
  `throbber-widgets-tui`.

## Testable
- Tests unitaires avec `ratatui::backend::TestBackend` : rendu du layout, saisie →
  `ToolRequest`, affichage d'une sortie avec secret masqué (`***`), `Esc` → fin de
  session, restauration du terminal au drop.
- Tests e2e scriptés : spawn du vrai binaire (`env!("CARGO_BIN_EXE_le-harness")`)
  avec clavier simulé (PTY ou événements injectés) dans un `tempfile` avec
  `trusted.txt` + `sensitives.txt` : session complète (démarrage, appel outil via
  `dummy-tool`, checkpoint, quit), sortie masquée d'un secret, exit 2 quand le repo
  n'a pas de politique, exit 2 quand `--repo` n'existe pas.
- Gate PR vert : `cargo fmt --check`, `clippy -D warnings`, `cargo test` complet,
  `cargo deny` + `cargo audit` sur les nouvelles dépendances.

## Hors scope
- Persistance fichier des checkpoints (étape 5), transport ACP (étape 6),
  connecteur la-console réel (étape 7), orchestrateur GitHub App (étape 8).
- Souris avancée, thèmes personnalisés, images (`ratatui-image`), mode inline
  (non plein écran), `reedline` (ligne de commande hors plein écran — autre mode).
- `tuirealm` (framework composant au-dessus de ratatui) : réévalué si l'UI grossit
  beaucoup. Async / `event-stream` : non, sauf besoin mesuré
  (`spec/ecosystem.md` §8).

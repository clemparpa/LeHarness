# Tour d'horizon de l'écosystème Rust — stack retenue (nov. 2026)

Veille menée en novembre 2026 pour choisir les librairies de l'étape 9 (TUI) et
trancher la question synchrone vs async, avant d'écrire `spec/steps/09-tui.md`.
Verdict : prendre / ne pas prendre / à évaluer, par brique. Sources en fin de
document.

## 1. UI terminal (TUI)

| Crate | Rôle | Statut | Verdict | Pourquoi |
|---|---|---|---|---|
| [ratatui](https://ratatui.rs) 0.30.x | Rendu TUI (widgets, layout, diff de frames) | ✅ très actif, fork communautaire de `tui-rs`, ~19k★, workspace modulaire depuis 0.30 | **PRENDRE** | De facto standard. Immediate-mode, backends pluggables, `ratatui::init()/run()` installent un panic hook qui restaure le terminal, `TestBackend` pour les tests |
| [crossterm](https://crates.io/crates/crossterm) 0.29 | Backend terminal (input, raw mode, alternate screen) | ✅ maintenu, cross-platform (Windows 7+ inclus) | **PRENDRE** (backend par défaut de ratatui) | Pur Rust. Boucle synchrone `event::poll(timeout)` + `read()` — pile ce qu'il faut sans runtime async |
| `tui-rs` | l'ancêtre | ❌ archivé | **NE PAS PRENDRE** | Advisory RustSec 2023-0049 : « unmaintained, use ratatui » |
| `cursive` | TUI retained-mode (style ncurses) | ⚠️ legacy | **NE PAS PRENDRE** | Pourrait sembler rapide à prendre, mais doc faible pour les vues custom et écosystème en déclin |
| `termion` 4.x | backend Unix-only | ⚠️ maintenance lente, pas de Windows | **NE PAS PRENDRE** | crossterm le domine |
| `termwiz` | backend (projet wezterm) | ⚠️ API instable, niche Windows | **NE PAS PRENDRE** | ratatui utilise crossterm par défaut |
| [tuirealm](https://crates.io/crates/tuirealm) v4 | framework React/Elm au-dessus de ratatui | ✅ maintenu | **À ÉVALUER — non retenu en v1** | Couche d'abstraction en plus ; pour une TUI de base sur notre cœur, ratatui pur suffit. À reconsidérer si l'UI grossit beaucoup |
| `blessed-rs` / `neo-blessed` | port JS de blessed | ⚠️ maintenance limitée | **NE PAS PRENDRE** | |
| `egui`, `iced`, `slint` | GUI desktop | ✅ actifs | **HORS SCOPE** | Pas des TUI — autre projet si un jour besoin desktop |
| `opentui_rust`, FrankenTUI | moteurs de rendu neufs | ❌ trop jeunes | **NE PAS PRENDRE** | |
| Widgets : [ratatui-textarea](https://github.com/ratatui/ratatui-textarea), `tui-input`, `throbber-widgets-tui`, `ratatui-image` | saisie multi-ligne, input, spinner, images | ✅ liste curatée [awesome-ratatui](https://github.com/ratatui/awesome-ratatui) | **PRENDRE au besoin** | textarea pour la zone de prompt, throbber pour l'activité |
| [reedline](https://github.com/nushell/reedline) | éditeur de ligne style readline/zle | ✅ très actif (nushell) | **PRENDRE si** ligne de commande « shell-like » hors plein écran | Incompatible avec un plein écran ratatui (deux gestionnaires du terminal) : choisir selon le mode |
| `dialoguer` / `inquire` | prompts simples hors TUI | ✅ | **Optionnel** | Pour la CLI `run` (étape 4) si besoin |

## 2. Boucle d'événements, signaux, erreurs (couche TUI)

| Choix | Recommandation | Pourquoi |
|---|---|---|
| Event loop | **Synchrone** : thread dédié `event::poll(tick)` + `read()` → `std::sync::mpsc::channel` → boucle principale (pattern officiel du tutoriel ratatui) | Cohérent avec la décision synchrone (§8). Pas de tokio dans la TUI v1 |
| Async (`event-stream`, `tokio::select!`) | ❌ en v1 | `event-stream` tire `futures` + un runtime ; n'apporte que du confort syntaxique |
| Signaux | [signal-hook](https://crates.io/crates/signal-hook) pour SIGINT/SIGTERM → restore terminal | cf. discussion officielle ratatui #220 |
| Erreurs / panics | [color-eyre](https://crates.io/crates/color-eyre) installé **avant** `ratatui::init()`/`run()` | Recette officielle ratatui : le panic hook restaure le terminal avant d'afficher le rapport |
| Flux de rendu | Backend sur **stderr** ; stdout réservé au protocole ligne-JSON vers l'extérieur | Ne pollue pas le canal outil |

## 3. Protocoles de transport

| Protocole / crate | Rôle | Statut | Verdict pour LeHarness |
|---|---|---|---|
| **ACP** (Zed/JetBrains) — crate [agent-client-protocol](https://crates.io/crates/agent-client-protocol) 1.10.x + [rust-sdk](https://github.com/agentclientprotocol/rust-sdk) | éditeur ↔ agent, JSON-RPC 2.0 sur stdio | Spec **v1 stable** (oct. 2026), Zed + JetBrains natifs, registre public d'agents (janv. 2026), 42+ agents / 19 éditeurs | **OUI, étape 6, couche optionnelle** — mais le SDK est **async** (futures + tokio pour process/stdio). En v1 : rôle `Agent` maison **synchrone** au-dessus de notre protocole ligne-JSON (§8) |
| **MCP** — crate [rmcp](https://docs.rs/rmcp) 3.6 (SDK Rust officiel) | agent ↔ outils externes MCP | spec 2026-07-28, basé sur **tokio** | **À considérer plus tard** si le harness appelle des serveurs MCP ; pas prioritaire vs la-console (étape 7) |
| **Notre protocole ligne-JSON** (`process.rs`, étape 3) | harness ↔ outils internes (la-console…) | maison, synchrone, testé, fuzzé | **GARDER tel quel** — canal interne fail-closed ; ACP/rmcp ne le remplacent pas |
| [jsonrpsee](https://github.com/paritytech/jsonrpsee) | JSON-RPC générique (WS/HTTP) | ✅ maintenu, **tokio**, lourd | **NE PAS PRENDRE** pour du stdio local — notre ligne-JSON suffit et est plus auditable |
| **AG-UI** (CopilotKit) / **A2UI** (Google) | agent ↔ UI web (SSE/WS) | actifs | **HORS SCOPE** — on est local/terminal |
| **A2A** (Linux Foundation, v1.0 mars 2026) | agent ↔ agent | standard émergent | **HORS SCOPE v1** (l'ancien « ACP » d'IBM est mort, fondu dans A2A — ne pas confondre avec l'ACP de Zed) |

## 4. Session management & persistance

Aucune librairie mature ne fait notre modèle (cœur de session + hooks + persistance
par trait injecté + fail-closed). Les crates existantes (`ai-session`, `lc-sessions`,
`adk-session`, `hermes-session-db`, `oneai-persistence`, `pond-db`) sont jeunes et
opinionated. Les vrais harnesses font leur propre stockage : Codex CLI et Claude Code
en **JSONL** par session (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` + index
SQLite), Pi en JSONL, OpenCode en **SQLite unique** (`opencode.db` depuis v1.2.0).
ACP laisse délibérément le format d'historique à l'agent.

| Option | Verdict | Pourquoi |
|---|---|---|
| **Cœur maison** (`session.rs` + hooks + trait de persistance) | **GARDER — c'est le cœur du projet** | Personne ne fait ce modèle + sécurité ; décision fondatrice |
| **JSON / JSONL** (serde_json, déjà là) | **PRENDRE pour la v1** (étape 5) | Append-only, lisible avec `jq`, même format que Codex/Claude Code/Pi |
| [redb](https://crates.io/crates/redb) | **PRENDRE si le volume le justifie** | Pure Rust, style LMDB, ACID, crash-safe, maintenu — le standard actuel |
| `sled` | **NE PAS PRENDRE** | Mode maintenance, réécriture v1 bloquée |
| `rusqlite` | **À ÉVALUER** si besoin de requêtes/index | Bundle C embarqué = surface d'attaque à auditer (pas d'`unsafe` côté nous, mais lib C dans le binaire) |
| `keyring` (keystore OS au lancement) | **À DISCUTER** | Amélioration de posture possible, mais sort de la règle « jamais de fichier » — décision à prendre avec la règle du broker |

## 5. Secrets, sous-processus, sandbox

| Option | Verdict | Pourquoi |
|---|---|---|
| [secrecy](https://docs.rs/secrecy) + [zeroize](https://crates.io/crates/zeroize) dans `secrets.rs` | **PRENDRE** (durcissement étape 2) | `SecretBox` : pas de `Debug`/`Display`/serde par défaut, wipe-on-drop. Renforce sans changer le modèle |
| `cryptex`, `secretspec`, `encryptman-keyring` | **NE PAS PRENDRE** | Trop opinionated — notre `SecretStore` mémoire-only reste la règle absolue |
| **Broker phantom token maison** (inspiré de [nono](https://nono.sh)) | **GARDER maison** | nono = référence d'architecture (proxy loopback, token 256 bits de session, validation constant-time, injection au boundary, `*_BASE_URL` dans l'env du fils) ; on ne prend pas sa lib, notre implémentation reste fail-closed et testée |
| `std::process` + `process.rs` maison (étape 3) | **GARDER** | Spawn + ligne-JSON + timeout + kill + fail-closed, déjà fuzzé. `command-group`/`nix` uniquement si besoin de tuer tout le process group |
| [landlock](https://crates.io/crates/landlock) (Linux 5.13+) / `sandbox-exec` (macOS Seatbelt) | **À ÉVALUER en v2** | Defense in depth côté harness, en complément de la cage userns+chroot de LaConsole ; kernel-enforced, unprivileged, irréversible |

## 6. Stack retenue pour l'étape 9 (TUI)

- `ratatui` 0.30 + `crossterm` 0.29 (backend par défaut) ; boucle **synchrone**
  (`poll`/`read` + thread d'événements → `mpsc`) ; rendu sur **stderr**.
- `color-eyre` (installé avant `ratatui::init()/run()`) + `signal-hook`.
- Widgets au besoin : `ratatui-textarea` (saisie), `throbber-widgets-tui` (activité) ;
  `TestBackend` pour les tests.
- Cœur inchangé : la TUI est une couche optionnelle, le cœur tourne sans.
- Détail de l'étape : [steps/09-tui.md](steps/09-tui.md).

## 7. ACP : toujours d'actualité ?

Oui — plus que jamais : spec **v1 stable** (oct. 2026), créée par Zed (août 2025),
co-développée avec JetBrains (natif dans les IDE JetBrains depuis déc. 2025),
registre public d'agents (janv. 2026), 42+ agents et 19 éditeurs compatibles,
JSON-RPC 2.0 sur stdio. L'étape 6 (ACP) reste au programme comme couche de
transport optionnelle. Seule nuance : le SDK Rust officiel est **async** — voir §8
pour la conséquence (rôle `Agent` maison synchrone en v1).

## 8. Décision : synchrone par défaut

**Position : le harness reste 100 % synchrone.** Aucun runtime async (`tokio` & co)
en dépendance directe. C'est cohérent avec la position déjà actée côté la-console
(`clemparpa/LaConsole`, spec/structure.md).

### Ce qui défend le choix synchrone

1. **Le protocole est mono-appelant, ligne par ligne.** Le cœur sert une requête à
   la fois (stdin → session → outil). L'async n'achète que de la concurrence d'I/O
   entrantes — il n'y en a pas à paralléliser.
2. **Surface d'attaque minimale = code déterministe.** Menace : agent/outil
   potentiellement compromis. Un runtime async introduit état partagé, tâches
   parallèles, timers, canaux : la combinatoire d'interleavings explose et chaque
   tâche parallèle devrait appliquer les mêmes bornes. En synchrone, l'exécution est
   séquentielle et reproductible : audit simple, tests déterministes (fuzz/mutants
   sans flakiness de timers).
3. **Pas de besoin mesuré.** Les I/O dominantes sont des sous-processus externes,
   déjà gérés par `process.rs` (spawn bloquant + timeout via thread + `recv_timeout`
   + kill du fils) et du HTTP bloquant. Un runtime n'accélère pas un appel bloquant
   externe : gagner exigerait de réécrire toute la pile réseau en async.
4. **La TUI n'a pas besoin d'async.** ratatui + crossterm en boucle synchrone est le
   pattern officiel ; `event-stream`/`tokio::select!` n'apportent que du confort
   syntaxique, au prix de `futures` + runtime.
5. **Supply chain et binaire allégés.** `tokio` + `mio` + co = deps lourdes, temps de
   compile, surface CVE/audit (on a pinact + min_age + `cargo deny`/`audit` +
   `cargo shear` : chaque grosse dépendance est un risque). Le `Cargo.lock` actuel
   n'a même pas `tokio`.
6. **Kill/timeout déjà résolus sans async** : thread + `recv_timeout` + kill du
   process (étape 3), avec proptest « aucun fils ne reste vivant après timeout/kill ».

### Pourquoi on ne passerait pas à l'async (critères de réouverture)

On ne passe à l'async que sur **besoin mesuré** (benchmark), et alors **couche par
couche, jamais dans le cœur** :

- plusieurs sessions concurrentes servies par un même cœur (orchestrateur
  multi-sessions — étape 8, dépriorisée) ;
- fan-out réseau parallèle à latence dominante (plusieurs appels MCP/ACP/HTTP
  simultanés) où le bloquant sérialise et dégrade la latence mesurée ;
- UI devant rester réactive pendant un traitement long *in-process* — et même là,
  un thread worker + `mpsc` suffit (c'est déjà le pattern TUI) ;
- adoption d'un SDK officiel async (ACP) : le runtime serait alors isolé dans la
  seule couche transport concernée.

## 9. Sources principales

- [ratatui](https://ratatui.rs) / [docs.rs ratatui](https://docs.rs/ratatui) / [awesome-ratatui](https://github.com/ratatui/awesome-ratatui) / recettes [color-eyre](https://ratatui.rs/recipes/apps/color-eyre/)
- [crossterm](https://crates.io/crates/crossterm), [RustSec 2023-0049 (tui archivé)](https://rustsec.org/advisories/RUSTSEC-2023-0049.html)
- [agentclientprotocol.com](https://agentclientprotocol.com) / [rust-sdk](https://github.com/agentclientprotocol/rust-sdk) / [docs.rs agent-client-protocol](https://docs.rs/agent-client-protocol) / [zed.dev/acp](https://zed.dev/acp)
- [rmcp (SDK MCP officiel)](https://docs.rs/rmcp), [jsonrpsee](https://github.com/paritytech/jsonrpsee)
- [nono](https://nono.sh) (phantom token, Landlock/Seatbelt), [crates.io nono](https://crates.io/crates/nono), [landlock](https://crates.io/crates/landlock)
- [redb](https://crates.io/crates/redb), [secrecy](https://docs.rs/secrecy), [zeroize](https://crates.io/crates/zeroize), [keyring](https://crates.io/crates/keyring), [reedline](https://github.com/nushell/reedline), [color-eyre](https://crates.io/crates/color-eyre), [signal-hook](https://crates.io/crates/signal-hook)
- Stockage des sessions des harnesses existants : Codex CLI (JSONL `~/.codex/sessions/` + index SQLite), Claude Code (JSONL), Pi (JSONL `~/.pi/agent/sessions/`), OpenCode (SQLite unique `~/.local/share/opencode/opencode.db`)

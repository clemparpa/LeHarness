# Tour d'horizon de l'écosystème Rust — stack retenue (nov. 2026)

Veille menée en novembre 2026 pour choisir les librairies de l'étape TUI et
trancher la question synchrone vs async. **Révision même jour** : la concurrence
(orchestration multi-sessions + appels d'outils en parallèle) est actée comme
**goal prioritaire** — la §8 est révisée en conséquence : on accepte la
concurrence, sans runtime async. Verdicts par brique : prendre / ne pas prendre /
à évaluer. Sources en fin de document.

## 1. UI terminal (TUI)

| Crate | Rôle | Statut | Verdict | Pourquoi |
|---|---|---|---|---|
| [ratatui](https://ratatui.rs) 0.30.x | Rendu TUI (widgets, layout, diff de frames) | ✅ très actif, fork communautaire de `tui-rs`, ~19k★, workspace modulaire depuis 0.30 | **PRENDRE** | De facto standard. Immediate-mode, backends pluggables, `ratatui::init()/run()` installent un panic hook qui restaure le terminal, `TestBackend` pour les tests |
| [crossterm](https://crates.io/crates/crossterm) 0.29 | Backend terminal (input, raw mode, alternate screen) | ✅ maintenu, cross-platform (Windows 7+ inclus) | **PRENDRE** (backend par défaut de ratatui) | Pur Rust. Boucle `event::poll(timeout)` + `read()` + thread d'événements → `mpsc` : zéro runtime async |
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
| Event loop | Thread dédié `event::poll(tick)` + `read()` → `std::sync::mpsc::channel` → boucle principale (pattern officiel ratatui). **Le même canal multiplexe aussi les résultats d'appels d'outils parallèles** (un worker thread par appel) | Zéro runtime async ; la concurrence passe par des threads OS bornées, pas par `async` |
| Async (`event-stream`, `tokio::select!`) | ❌ | `event-stream` tire `futures` + un runtime ; n'apporte que du confort syntaxique |
| Signaux | [signal-hook](https://crates.io/crates/signal-hook) pour SIGINT/SIGTERM → restore terminal | cf. discussion officielle ratatui #220 |
| Erreurs / panics | [color-eyre](https://crates.io/crates/color-eyre) installé **avant** `ratatui::init()`/`run()` | Recette officielle ratatui : le panic hook restaure le terminal avant d'afficher le rapport |
| Flux de rendu | Backend sur **stderr** ; stdout réservé au protocole ligne-JSON vers l'extérieur | Ne pollue pas le canal outil |

## 3. Protocoles de transport

| Protocole / crate | Rôle | Statut | Verdict pour LeHarness |
|---|---|---|---|
| **ACP** (Zed/JetBrains) — crate [agent-client-protocol](https://crates.io/crates/agent-client-protocol) 1.10.x + [rust-sdk](https://github.com/agentclientprotocol/rust-sdk) | éditeur ↔ agent, JSON-RPC 2.0 sur stdio | Spec **v1 stable** (oct. 2026), Zed + JetBrains natifs, registre public d'agents (janv. 2026), 42+ agents / 19 éditeurs | **OUI, étape 6, couche optionnelle** — mais le SDK est **async** (futures + tokio pour process/stdio). En v1 : rôle **`Proxy`** maison (entre éditeur client et agent LLM externe — le harness ne parle jamais au modèle) au-dessus de notre protocole ligne-JSON, sans runtime async (§8) |
| **MCP** — crate [rmcp](https://docs.rs/rmcp) 3.6 (SDK Rust officiel) | agent ↔ outils externes MCP | spec 2026-07-28, basé sur **tokio** | **À considérer plus tard** si le harness appelle des serveurs MCP ; pas prioritaire vs la-console (étape 7) |
| **Notre protocole ligne-JSON** (`process.rs`, étape 3) | harness ↔ outils internes (la-console…) et orchestrateur ↔ session | maison, synchrone, testé, fuzzé | **GARDER tel quel** — canal interne fail-closed, et base de l'orchestration multi-sessions (un processus par session) |
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
| **Broker phantom token maison** (inspiré de [nono](https://nono.sh)) | **GARDER maison, par session** | nono = référence d'architecture (proxy loopback, token 256 bits de session, validation constant-time, injection au boundary). Avec le multi-sessions : **un phantom token par session**, dans le processus de la session uniquement |
| `std::process` + `process.rs` maison (étape 3) | **GARDER, c'est aussi le mécanisme de concurrence** | Un processus par session (orchestrateur) et un thread + sous-processus par appel d'outil parallèle ; timeout + kill + fail-closed déjà fuzzés. `command-group`/`nix` uniquement si besoin de tuer tout le process group |
| [landlock](https://crates.io/crates/landlock) (Linux 5.13+) / `sandbox-exec` (macOS Seatbelt) | **À ÉVALUER en v2** | Defense in depth côté harness, en complément de la cage userns+chroot de LaConsole ; kernel-enforced, unprivileged, irréversible |

## 6. Stack retenue (étapes 8, 9)

- **Étape 8 — orchestrateur multi-sessions** : un **processus par session** via
  `process.rs`, un thread lecteur par session → `mpsc` ; bornes `--max-sessions`,
  flock par repo, rlimits, timeout par message. Zéro dépendance ajoutée.
- **Étape 9 — TUI** : `ratatui` 0.30 + `crossterm` 0.29 (boucle `poll`/`read` +
  thread d'événements → `mpsc`, rendu sur **stderr**), `color-eyre` +
  `signal-hook`. Widgets au besoin : `ratatui-textarea`, `throbber-widgets-tui` ;
  `TestBackend` pour les tests.
- **Étape 9 aussi — appels d'outils en parallèle (bornés)** dans le cœur :
  `submit` non bloquant → worker threads (plafond `--max-parallel-tools`,
  défaut 4), résultats taggés par id de requête dans le même `mpsc` que les
  événements clavier ; les appels avec redirection (`>`/`>>`) sont sérialisés.
- Cœur inchangé dans son modèle : la TUI et l'orchestrateur sont des couches
  optionnelles, le cœur tourne sans.
- Détails : [steps/08-multi-session.md](steps/08-multi-session.md) et
  [steps/09-tui.md](steps/09-tui.md).

## 7. ACP : toujours d'actualité ?

Oui — plus que jamais : spec **v1 stable** (oct. 2026), créée par Zed (août 2025),
co-développée avec JetBrains (natif dans les IDE JetBrains depuis déc. 2025),
registre public d'agents (janv. 2026), 42+ agents et 19 éditeurs compatibles,
JSON-RPC 2.0 sur stdio. L'étape 6 (ACP) reste au programme comme couche de
transport optionnelle. Seule nuance : le SDK Rust officiel est **async** — voir §8
pour la conséquence (rôle `Proxy` maison, sans runtime async, en v1 : le harness
est un proxy entre l'éditeur et l'agent LLM externe, pas l'agent lui-même).

## 8. Décision révisée : concurrence bornée, sans runtime async

**Position (nov. 2026, révisée) : la concurrence est un goal acté** —
orchestration multi-sessions (serveur qui traite plusieurs sessions en même
temps) et appels d'outils en parallèle (rapidité de résolution de l'agent).
Elle se réalise **sans runtime async** : un processus par session, des threads
bornées par appel d'outil. « Synchrone » signifiait « pas de runtime async », pas
« pas de concurrence ». Aucun `tokio`/`async` en dépendance directe.

### Pourquoi des processus/threads bornés plutôt qu'un runtime async

1. **Les unités de concurrence sont lourdes.** Une session = un agent + des
   outils ; un appel d'outil = un sous-processus qu'on attend de toute façon
   (attente bloquante inhérente). Un thread/processus par unité est le grain
   naturel : l'async n'enlèverait pas l'attente bloquante, il la déplacerait.
2. **Isolation par processus = domaine de confiance par session.** Chaque session
   a son propre `SecretStore`, son propre phantom token, ses propres bornes ; un
   crash ou un hang d'une session ne touche pas les autres (même principe que
   « la console est un sous-processus »). En async avec état partagé, toute la
   mémoire — secrets inclus — serait commune à toutes les sessions.
3. **Déterminisme et audit par unité.** Chaque session reste séquentielle et
   reproductible en interne ; les bornes (timeout, kill, masquage, quota) s'appliquent
   par unité. L'orchestrateur multiplexe les réponses ligne-JSON via un thread
   lecteur par session poussant dans un `mpsc` : de la concurrence OS simple,
   auditable, testable (wall-clock) — pas d'interleavings d'un runtime.
4. **Bornes explicites, obligatoires dès qu'on est concurrent** :
   - multi-sessions : `--max-sessions N` global (refus au-delà, fail-closed),
     **flock par repo** (jamais deux sessions sur le même repo), rlimits sur
     chaque fils, timeout par message + kill propre, log d'audit append-only par
     session ; l'orchestrateur ne détient **aucun secret de session** (il route) ;
   - outils parallèles : `--max-parallel-tools N` (défaut 4, plafond 16) ; les
     appels contenant une redirection (`>`/`>>`) sont **sérialisés** (mutex) pour
     éviter les courses d'écriture ; les appels read-only tournent en parallèle ;
     chaque appel garde timeout + kill + masquage ; le quota disque total est borné
     à N × quota par commande.
5. **Supply chain inchangée** : `std::thread` + `std::process`, zéro dépendance
   ajoutée — le `Cargo.lock` reste sans `tokio`.

### 8.1 Ce que font les harnais Rust existants (nov. 2026)

| Harnais Rust | Substrat async ? | Pourquoi ce choix chez eux |
|---|---|---|
| [Codex CLI](https://github.com/openai/codex) (codex-rs) | **tokio** partout (`tokio::process::Command` pour les subprocess sandboxés ; TUI ratatui+crossterm+tokio) | Il **parle au modèle en HTTP streaming** + PTY + multi-I/O : la latence réseau domine, l'async paie |
| [Goose](https://github.com/block/goose) (Block/AAIF) | **tokio** (`tokio::process`, `tokio::net::TcpListener` en mode web) | Providers LLM en HTTP + serveurs MCP (stdio *et* HTTP) : beaucoup d'I/O réseau concurrent |
| [jcode](https://github.com/1jehuang/jcode) | runtime async + **un seul daemon partagé** par toutes les sessions (~28 Mo PSS total, sessions en mémoire partagée) | Multi-sessions efficace en mémoire — compromis inverse du nôtre : nous, **isolation par processus** (les secrets ne sont jamais en mémoire partagée entre sessions) |
| [pi_agent_rust](https://github.com/Dicklesworthstone/pi_agent_rust) | runtime async maison (« asupersync »), **threads dédiés stdin/stdout pontés par canaux** vers le runtime | Même constat : boucle agent (LLM streaming) en async, I/O terminal en threads |
| [rig_cat](https://docs.rs/rig-cat) | **pas d'async** (« No async, no tokio » — fibers + threads) | Contre-exemple de niche : composition synchrone, parallélisme par threads |
| [Rig](https://rig.rs) | tokio dans les exemples, mais le cœur **ne requiert pas tokio** | Core `Send + Sync`, runtime au choix de l'appelant |

**Lecture** : tous les harnais Rust qui ont choisi l'async le font parce qu'ils
appellent des **API LLM en HTTP streaming** — leur attente dominante est le
réseau. LeHarness ne parle **pas au modèle** (c'est l'agent externe qui le fait,
voir `structure.md`) : ses attentes dominantes sont des **sous-processus**
(la-console) et des fichiers locaux — et même tokio fait son file I/O en
bloquant sous le capot (chaque appel `tokio::fs` spawn un thread). Pour ce
profil de charge, `std::thread` est le bon grain ; l'async n'achète qu'une
couche d'interleavings. Les seules couches réseau (ACP, MCP-over-HTTP, webhooks
GitHub App) sont précisément celles où un runtime async serait justifié —
optionnelles, isolées, jamais dans le cœur.

### Quand un runtime async resterait envisageable

Uniquement sur **besoin mesuré**, et isolé dans une couche réseau dédiée (jamais
le cœur) : front réseau à très haute densité de connexions (C10k), ou fan-out
HTTP massif avec latence dominante mesurée. Tant que les sessions restent des
processus lourds, ce n'est pas notre cas.

### 8.2 Tranche finale (nov. 2026 — clarifié : le harness est un proxy ACP)

Cas d'usage tranchés :

- **Plusieurs sessions en même temps** (lancer plusieurs tâches) → côté harness,
  étape 8 `serve` : **un processus par session**, thread lecteur par session →
  `mpsc`, bornes fail-closed (`--max-sessions`, flock par repo, rlimits, timeout
  par message). Zéro async.
- **Appels d'outils en parallèle** : l'agent externe peut en émettre plusieurs
  d'un coup ; le harness doit donc accepter plusieurs requêtes **en vol** → cœur
  étape 9 : `submit` non bloquant + worker threads bornées
  (`--max-parallel-tools`, défaut 4, plafond 16), résultats taggés par id via
  `mpsc`, redirections (`>`/`>>`) sérialisées par mutex, timeout + kill +
  masquage par appel. Zéro async.
- **Runtime async** : exclu du cœur. Il n'entrera que dans une couche réseau
  isolée et optionnelle (webhooks GitHub App en v2, MCP-over-HTTP) sur besoin
  mesuré — le seul endroit où l'I/O réseau deviendrait dominant.

## 9. Sources principales

- [ratatui](https://ratatui.rs) / [docs.rs ratatui](https://docs.rs/ratatui) / [awesome-ratatui](https://github.com/ratatui/awesome-ratatui) / recettes [color-eyre](https://ratatui.rs/recipes/apps/color-eyre/)
- [crossterm](https://crates.io/crates/crossterm), [RustSec 2023-0049 (tui archivé)](https://rustsec.org/advisories/RUSTSEC-2023-0049.html)
- [agentclientprotocol.com](https://agentclientprotocol.com) / [rust-sdk](https://github.com/agentclientprotocol/rust-sdk) / [docs.rs agent-client-protocol](https://docs.rs/agent-client-protocol) / [zed.dev/acp](https://zed.dev/acp)
- [rmcp (SDK MCP officiel)](https://docs.rs/rmcp), [jsonrpsee](https://github.com/paritytech/jsonrpsee)
- [nono](https://nono.sh) (phantom token, Landlock/Seatbelt), [crates.io nono](https://crates.io/crates/nono), [landlock](https://crates.io/crates/landlock)
- [redb](https://crates.io/crates/redb), [secrecy](https://docs.rs/secrecy), [zeroize](https://crates.io/crates/zeroize), [keyring](https://crates.io/crates/keyring), [reedline](https://github.com/nushell/reedline), [color-eyre](https://crates.io/crates/color-eyre), [signal-hook](https://crates.io/crates/signal-hook)
- Stockage des sessions des harnesses existants : Codex CLI (JSONL `~/.codex/sessions/` + index SQLite), Claude Code (JSONL), Pi (JSONL `~/.pi/agent/sessions/`), OpenCode (SQLite unique `~/.local/share/opencode/opencode.db`)

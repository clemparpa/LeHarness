# Structure du dépôt et dépendances

## Structure

```
LeHarness/
  Cargo.toml          # workspace root (virtual manifest, un seul Cargo.lock)
  crates/
    harness/          # le harness (binaire + lib)
      Cargo.toml
      src/
        main.rs       # CLI clap : sous-commandes (run --repo, ...)
        lib.rs        # #![forbid(unsafe_code)]
        session.rs    # Session + hooks (on_start/on_checkpoint/on_end/on_resume)
        secrets.rs    # SecretStore : memoire uniquement, refresh, masquage ***
        process.rs    # lanceur generique de sous-processus (ligne JSON), timeout, kill
        policy.rs     # politique du repo (trusted.txt / sensitives.txt), fail-closed
        run.rs        # boucle CLI local interactive (run --repo), testable hors main
        bin/
          dummy-tool.rs  # binaire outil bidon pour les tests e2e (CARGO_BIN_EXE_dummy-tool)
      tests/
        e2e.rs        # spawn du binaire reel + outil bidon
  fuzz/               # workspace cargo-fuzz, exclu du workspace principal
    fuzz_targets/
      parse_line.rs
      mask_secrets.rs
  tests/              # tests d'integration haut niveau (niveau workspace)
  docs/
  spec/
    steps/            # une spec par etape de la roadmap (NN-slug.md)
  ROADMAP.md
```

- Nommage : crates en kebab-case (`harness`) ; tout le code, fichiers et
  symboles en anglais ; la doc en français.
- **Aucune dépendance vers la-console** : la console est un binaire externe,
  lancé via `process.rs`. Aucun `use la_console` nulle part.

## Dépendances

**Règle : concurrence bornée, sans runtime async** — aucun runtime async
(`tokio` & co) en dépendance directe. La concurrence (multi-sessions =
processus par session, outils parallèles = threads bornées) n'ajoute **aucune**
dépendance : `std::thread` + `std::process`. Décision et bornes :
[ecosystem.md](ecosystem.md) §8. `cargo shear` vérifie qu'aucune dépendance
n'est inutilisée.

Actuelles (`Cargo.lock`) :

| Besoin | Crate |
|---|---|
| CLI | `clap` |
| Protocole JSON ligne par ligne | `serde`, `serde_json` |
| Erreurs typées | `thiserror` |
| Spawn de process + timeout + kill | `std::process` + thread + `recv_timeout` (maison, synchrone — **pas de tokio**) |
| Tests (répertoires temporaires) | `tempfile` (dev-dependency) |
| Property-based tests | `proptest` (dev-dependency) |

Prévues, par étape :

| Besoin | Crate | Étape |
|---|---|---|
| TUI : rendu + backend terminal | `ratatui` + `crossterm` | 9 |
| Erreurs / panic hook TUI (restore terminal), signaux | `color-eyre`, `signal-hook` | 9 |
| Widgets TUI (saisie, spinner) | `ratatui-textarea` (ou `tui-input`), `throbber-widgets-tui` | 9 |
| Secrets : newtype sans `Debug`/`Display` + wipe-on-drop | `secrecy` (+ `zeroize`) | durcissement étape 2 |
| Persistance locale (si le volume le justifie) | `redb` | 5 |
| Checksum / hachage | `sha2` | au besoin |
| Regex (fichiers sensibles) | `regex` (temps linéaire, pas de ReDoS) | au besoin |
| HTTP côté harness (orchestrateur), mode **blocking** | `reqwest` + rustls | 8 |
| Logs structurés | `tracing`, `tracing-subscriber` | au besoin |
| Transport ACP optionnel | **v1 : rôle `Agent` maison synchrone** au-dessus du protocole ligne-JSON ; le SDK officiel `agent-client-protocol` (async) reste l'option si besoin mesuré | 6 |

Notes :
- Cœur sécurité (`secrets.rs`, `process.rs`, hooks de session) : **maison**,
  volontairement.
- Aucun framework d'agents (Rig & co) : le harness ne parle pas au modèle,
  c'est l'agent externe qui le fait.

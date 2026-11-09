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

## Dépendances (figées pour la v1)

| Besoin | Crate |
|---|---|
| Async + timeout + sous-processus | `tokio` (`tokio::process`) |
| Spawn de process (synchrone de secours) | `std::process` (pas de crate) |
| Protocole JSON ligne par ligne | `serde`, `serde_json` |
| CLI | `clap` |
| HTTP côté harness (orchestrateur GitHub App) | `reqwest` + rustls |
| Checksum / hachage | `sha2` |
| Regex (fichiers sensibles) | `regex` (temps linéaire, pas de ReDoS) |
| Logs structurés | `tracing`, `tracing-subscriber` |
| Erreurs typées | `thiserror` |
| Persistance locale (étape 5, si volume le justifie) | `redb` |
| Transport ACP optionnel (étape 6) | `agent-client-protocol` (officiel) |
| Tests (répertoires temporaires) | `tempfile` (dev-dependency) |
| Property-based tests | `proptest` (dev-dependency) |

Notes :
- Cœur sécurité (`secrets.rs`, `process.rs`, hooks de session) : **maison**,
  volontairement.
- Aucun framework d'agents (Rig & co) : le harness ne parle pas au modèle,
  c'est l'agent externe qui le fait.

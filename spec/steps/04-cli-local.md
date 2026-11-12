# Étape 4 — CLI local

## Objectif
Une CLI clap `le-harness run --repo <chemin>` qui démarre une session sur un
repo local, lit la politique du repo (`trusted.txt`, `sensitives.txt`), et
boucle en interactif sur stdin (stdin → session → outil).

## Contrat / API
- `crates/harness/src/main.rs` : CLI clap avec la sous-commande
  `run --repo <chemin> [--tool <binaire>] [--secrets-file <chemin>] [--tool-timeout <secondes>]`.
  Sans sous-commande : affiche l'aide (exit 0 via clap). Toute erreur = exit 2
  (fail-closed), message sur stderr.
- `crates/harness/src/policy.rs` : `RepoPolicy { trusted: Vec<String>, sensitives: Vec<String> }`
  chargée depuis `<repo>/trusted.txt` et `<repo>/sensitives.txt`
  (`RepoPolicy::load(root) -> Result<RepoPolicy, PolicyError>`).
  Format : une entrée par ligne, lignes vides et commentaires `#` ignorés.
  **Fail-closed** : un fichier manquant ou illisible = `PolicyError`, la
  session ne démarre pas (aucune politique déclarée = refus, jamais une
  politique vide silencieuse).
- `crates/harness/src/run.rs` : la boucle, factorisée hors de `main` pour être
  testable :
  `run(config, input: impl BufRead, output: impl Write) -> Result<RunOutcome, RunError>`.
  - Démarre une `Session` (étape 1) sur le repo, avec persistance en mémoire
  (la persistance fichier arrive à l'étape 5) et des hooks qui écrivent les
  événements sur la sortie.
  - Charge le `SecretStore` depuis `--secrets-file` si fourni, sinon vide.
  - Boucle sur les lignes de stdin :
    - ligne vide = ignorée ;
    - `quit` / `exit` = `session.end()`, imprime `session <id> ended`, sortie ;
    - `checkpoint` = `session.checkpoint()`, imprime `checkpoint <n>` ;
    - toute autre ligne = envoyée comme `ToolRequest { id, input }` à l'outil
      `--tool` via le lanceur de l'étape 3 (timeout `--tool-timeout`, défaut
      30 s) ; la réponse `stdout`/`stderr` est ré-masquée par le `SecretStore`
      avant d'être imprimée ; une erreur outil est imprimée (`error: ...`) et
      la boucle continue ;
    - sans `--tool`, une ligne outil = `error: no tool configured` ;
    - EOF sur stdin = fin de session comme `quit`.
  - Au spawn de l'outil, les chemins des fichiers de politique sont transmis
  au binaire outil via les args `--trusted-file <path> --sensitives-file <path>`
  (le harness déclare la politique, l'outil l'applique — voir `spec/security.md`).
  - `RunOutcome { checkpoints: u64, tool_calls: u64 }` pour les tests.
- Aucune lecture clavier directe hors stdin ; synchrone de bout en bout.
- Aucun `unsafe`.

## Testable
- Tests unitaires `policy.rs` (avec `tempfile`) : chargement nominal,
  commentaires/lignes vides ignorés, fichier manquant = erreur, fichier
  illisible = erreur, repo inexistant = erreur.
- Tests unitaires `run.rs` (stdin/stdout en mémoire) : `checkpoint` puis
  `quit`, EOF = fin de session, ligne vide ignorée, outil absent = `error: no
  tool configured`, session démarrée sur le bon root.
- Tests e2e (`crates/harness/tests/e2e.rs`) contre le vrai binaire
  (`env!("CARGO_BIN_EXE_le-harness")`) dans un `tempfile` avec `trusted.txt` +
  `sensitives.txt` : session complète scriptée via stdin pipé (`checkpoint`,
  appel outil via `dummy-tool`, `quit`), sortie masquée d'un secret, exit 2
  quand le repo n'a pas de politique, exit 2 quand `--repo` n'existe pas.

## Hors scope
Persistance fichier des checkpoints (étape 5), transport ACP (étape 6),
connecteur la-console réel (étape 7), orchestrateur GitHub App (étape 8),
validation des regex de `sensitives.txt` (appliquée par l'outil, pas par le
harness).

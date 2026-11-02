# Fausse console (fake console) — design

Première brique du harness : une console volontairement très limitée et très sûre,
utilisable par un agent sans risque de sortie de son périmètre.

## Objectifs

1. **Périmètre absolu** : tout accès fichier est confiné sous un `root` donné.
   Aucune commande ne doit pouvoir lire/écrire hors de ce root, par quelque moyen
   que ce soit (`..`, symlinks, chemins absolus, encodages, etc.).
2. **Opérations limitées** : seules quelques commandes whitelistées sont disponibles.
3. **Commandes trusted** : commandes arbitraires définies par l'humain dans un
   fichier du repo que l'agent ne peut pas modifier. Elles ne sont **pas**
   interprétées par la console : elles sont passées telles quelles à un vrai
   shell, considérées comme sûres car validées par l'humain, avec checksum +
   scan des changements avant chaque exécution.

## Statut des décisions

| Sujet | Statut |
|---|---|
| Syntaxe des commandes whitelist proche du shell (bash) | ✅ décidé — pour que les modèles utilisent le tool au mieux |
| Commandes trusted : invoquées par nom (`trusted <nom>`), fichiers sensibles déclarés par regex, scan des changements entre chaque exécution | ✅ décidé |
| Session de la console | ✅ décidé — **stateless** : chaque commande repart du root, pas de `cd` persistant en v1 |
| Interface de la console | ✅ décidé — **sous-processus dédié**, protocole ligne par ligne sur stdin/stdout : une commande en entrée, un résultat en sortie. Isolation par processus : un crash de la console (ex. lecture d'un très gros fichier) ne fait pas planter le harness |
| Confinement | ✅ décidé — **v1 : logique pur Rust** (`resolve()` + canonicalize + tests d'évasion en CI), marche partout sans privilèges. **v2 (déploiement remote) : user namespace non privilégié + chroot** par-dessus (defense in depth, le noyau bloque même en cas de bug). Jamais de root global au lancement de l'agent. L'API reste identique, seul le lanceur change |
| Format des sorties/erreurs | ✅ décidé — **JSON structuré** : chaque réponse est un objet `{ok, code, stdout, stderr}`. Séparation explicite entre métadonnées (succès/échec) et contenu, moins de risque d'injection dans l'interprétation |

## Modèle de sécurité

### Confinement par path root

- Le `root` est résolu en chemin canonique **une seule fois** au démarrage
  (`std::fs::canonicalize`), puis toutes les opérations travaillent exclusivement
  avec des chemins relatifs au root.
- Toute résolution de chemin utilisateur passe par une fonction unique
  `resolve(root, input) -> Result<PathBuf, Denied>` qui :
  - rejette les chemins absolus (sauf s'ils sont sous le root canonique) ;
  - rejette tout segment `..` après normalisation logique (composants `ParentDir`) ;
  - **résout les symlinks** (`fs::canonicalize` sur le parent existant le plus proche,
    puis vérification que le résultat reste préfixe du root canonique) ;
  - rejette les hardlinks pointant hors du root (vérification via `st_dev`/`st_ino`
    ou, plus simplement, en refusant la création de liens et en suivant les liens
    existants uniquement après canonicalisation).
- Règle d'or : **jamais de `Path::join` direct sur l'input utilisateur** sans passer
  par `resolve`. Aucune exception.

### Whitelist de commandes (v1) — tableau récapitulatif de tout ce qui est autorisé

Règle absolue : **toute commande exécutée doit matcher la whitelist (ou trusted),
et tout chemin impliqué passe par `resolve()` + test fichiers sensibles**, même
après expansion de glob ou substitution de variable.

| Élément | Autorisé ? | Détail |
|---|---|---|
| `cat <path>` | ✅ | `resolve()` + taille max |
| `ls [path]` | ✅ | `resolve()`, entrées bornées |
| `echo <text> > <path>` | ✅ | `resolve()`, taille max, `>` seul opérateur de redirection |
| `grep <pattern> <path...>` | ✅ | via crate `grep`, fichiers bornés |
| `sub <pattern> <repl> <path>` (sed-like) | ✅ | substitution via `regex` uniquement, fichiers bornés |
| `&&` | ✅ | chaînage : exécution séquentielle, stop au 1er échec. **Les deux côtés doivent être whitelist ou trusted.** Jamais de vrai shell |
| `;` | ✅ | interprété comme `&&` (plus strict que bash : stop au 1er échec) — l'agent n'est pas dépaysé |
| pipes `\|` | ✅ (prévu) | **whitelist uniquement, jamais avec `trusted`** ; la console unwrap le `stdout` JSON de la commande de gauche et le passe en entrée de celle de droite |
| `>>`, `<`, `2>`, autres redirections | ⏳ | seul `>` de `echo` est permis en v1 ; les autres redirections seront discutées plus tard |
| globs `*`, `?`, `**` | ✅ | via `wax` ; **chaque fichier matché passe individuellement par `resolve()` + sensibles**, nb de matchs borné |
| variables `$VAR`, `${VAR}`, `$(VAR)` | ⚠️ restreint | uniquement des variables **déclarées par l'humain** (ex. `$ROOT`) ; `$(...)` accepté mais le contenu doit matcher `^[A-Za-z_][A-Za-z0-9_]*$` (nom de variable seule, jamais de sous-exécution) ; **pas d'affectation `FOO=bar` par l'agent**, pas de variables d'état interne (`$?`, `$!`, `$0`...) |
| backticks | ❌ | jamais de sous-exécution de commande |
| `trusted <nom>` | ✅ | commande du fichier `trusted.txt` du repo, avec checksum + scan demandé au harness |
| tout le reste | ❌ | `unknown command`, fail-closed |

### Parsing

- Tokenisation via `shell-words` (quotes `"..."` / `'...'` style bash), puis
  interprétation **maison et cadrée** des seuls opérateurs autorisés (`&&`,
  globs `wax`, variables déclarées). Aucun appel à `sh -c` / `bash -c` pour
  les commandes whitelistées.

## Commandes trusted

- Définies par l'humain dans un fichier dédié du repo (ex. `console/trusted.txt`),
  que l'agent ne peut pas modifier (revu en PR, protégé par review humaine).
- Invoquées par l'agent via `trusted <nom>` ; la console ne parse pas le contenu,
  elle le passe tel quel à un vrai shell avec `cwd = root` et un environnement
  réduit (variables d'env minimales).
- **Fichiers sensibles** : déclarés par des regex (ex. `.*\.env`, `.*secret.*`,
  `\.git/config$`), une par ligne dans un fichier du repo de l'agent. Le matching
  se fait sur le **chemin relatif au root** (plus portable, l'agent raisonne en
  relatif vu que la console est stateless). Le moteur `regex` est à temps linéaire :
  pas de ReDoS possible. Toute commande whitelist vers un chemin matchant est
  refusée ; toute commande trusted qui touche à un fichier matchant est soumise
  à un scan renforcé.
- Avant chaque exécution :
  1. **checksum** (sha256) du fichier de commandes trusted consigné ;
  2. **scan des changements** depuis la dernière exécution validée (diff du
     fichier + du code de test associé) pour vérifier qu'il n'y a pas de code
     adversarial introduit ;
  3. exécution uniquement si checksum attendu == checksum constaté et scan OK.

## Secrets (GitHub App, ex. token git)

- Le token reste **en mémoire dans le harness** et transite uniquement par
  **fd / socket unix** vers un **credential helper git externe**, placé hors du
  root et non lisible par l'agent.
- Jamais dans l'env du shell de la console (une commande trusted pourrait faire
  `printenv`), jamais en argument de ligne de commande (visible dans `ps`),
  jamais dans un fichier lisible par l'agent.
- `HOME` du sous-processus des commandes trusted pointé hors du root, avec une
  config git propre ; `.git/config` et `.git-credentials` sont des fichiers
  sensibles (regex).
- Toute occurrence du token dans `stdout`/`stderr` est masquée (`***`) avant
  l'envoi de la réponse JSON.
- Les tokens d'installation GitHub expirent après 1 h : le harness les refresh
  lui-même et fournit le token frais à chaque exécution.

## Structure du repo (monorepo, workspace Cargo)

```
LeHarness/
  Cargo.toml          # workspace root (virtual manifest, un seul Cargo.lock)
  crates/
    la-console/       # la fausse console (binaire + lib)
      Cargo.toml
      src/
        main.rs       # CLI clap : --root, bornes ; spawn du protocole JSON
        lib.rs
        console.rs    # boucle protocole JSON stdin/stdout (tokio)
        resolve.rs    # resolve() : confinement root, unique point de passage
        commands/     # cat, ls, echo, grep (whitelist)
        trusted.rs    # commandes trusted : parsing + wrapping + checksum + scan + exécution
      tests/
        evasion.rs    # tests d'évasion (.., symlinks, absolus, unicode...)
        commands.rs
  tests/              # tests d'intégration haut niveau + simulations (niveau workspace)
  docs/
```

- Nommage : noms de crates en kebab-case (`la-console`) ; tout le code, fichiers et
  symboles en anglais.
- **Commandes trusted** : le fichier `trusted.txt` (et les regex de fichiers
  sensibles) vit **dans chaque repo où tourne l'agent**, pas dans la-console.
  Le module `trusted.rs` de la console ne gère que le parsing, le wrapping,
  la sécu (checksum, scan des diffs) et l'exécution — il reçoit le chemin du
  fichier en argument.
- Le crate `harness` n'est pas créé pour l'instant ; on s'y attaquera quand on
  travaillera sur le harness.

## Dépendances (figées pour la v1)

| Besoin | Crate |
|---|---|
| Protocole JSON stdin/stdout | `serde`, `serde_json` |
| Async + timeout par commande | `tokio` |
| `grep` dans des fichiers | `grep` (moteur ripgrep en lib) |
| `ls` (respecte .gitignore, bornes) | `ignore` (walker ripgrep) |
| Checksum sha256 des commandes trusted | `sha2` |
| Regex des fichiers sensibles | `regex` (temps linéaire, pas de ReDoS) |
| Limites mémoire/CPU du sous-processus | `rlimit` |
| Logs structurés | `tracing`, `tracing-subscriber` |
| Erreurs typées | `thiserror` |
| CLI de la console | `clap` |
| Tokenisation shell (quotes, espaces) | `shell-words` |
| Globs (`*.rs`, `**/*.md`) | `wax` (moteur de glob de ripgrep) |
| Tests (répertoires temporaires) | `tempfile` (dev-dependency) |

Notes :
- `reqwest` (HTTP, refresh du token GitHub App) reste **côté harness**, jamais dans la console.
- Spawn du sous-processus et pipes : `std::process` suffit, pas de crate.
- Cœur sécurité (`resolve()`, parser whitelist) : **maison**, volontairement.

## Principes

- Fail-closed : au doute, refuser.
- Une seule fonction de résolution de chemins, testée agressivement.
- Aucun `unsafe` dans la v1.
- Pas de dépendances réseau dans la console.

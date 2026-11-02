# Fausse console (fake console) — design

Première brique du harness : une console volontairement très limitée et très sûre,
utilisable par un agent sans risque de sortie de son périmètre.

## Objectifs

1. **Périmètre absolu** : tout accès fichier est confiné sous un `root` donné.
   Aucune commande ne doit pouvoir lire/écrire hors de ce root, par quelque moyen
   que ce soit (`..`, symlinks, chemins absolus, encodages, etc.).
2. **Opérations limitées** : seules quelques commandes whitelistées sont disponibles.
3. **Commandes pures** : commandes arbitraires définies par l'humain dans un
   fichier du repo que l'agent ne peut pas modifier. Elles ne sont **pas**
   interprétées par la console : elles sont passées telles quelles à un vrai
   shell, considérées comme sûres car validées par l'humain, avec checksum +
   scan des changements avant chaque exécution.

## Statut des décisions

| Sujet | Statut |
|---|---|
| Syntaxe des commandes whitelist proche du shell (bash) | ✅ décidé — pour que les modèles utilisent le tool au mieux |
| Commandes pures : invoquées par nom (`pure <nom>`), fichiers sensibles déclarés par regex, scan des changements entre chaque exécution | ✅ décidé |
| Session de la console | ✅ décidé — **stateless** : chaque commande repart du root, pas de `cd` persistant en v1 |
| Interface de la console | ✅ décidé — **sous-processus dédié**, protocole ligne par ligne sur stdin/stdout : une commande en entrée, un résultat en sortie. Isolation par processus : un crash de la console (ex. lecture d'un très gros fichier) ne fait pas planter le harness |
| Confinement : logique (`resolve()` pur Rust) vs dur (chroot / user namespace / sandbox OS) | ⏳ à trancher |
| Format des sorties et erreurs (texte brut vs JSON, codes de sortie) | ⏳ à trancher |

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

### Whitelist de commandes (v1)

Syntaxe volontairement proche du shell bash, pour que les agents la reconnaissent.

| Commande | Effet | Contraintes |
|---|---|---|
| `cat <path>` | lit un fichier | `resolve` + taille max (ex. 1 Mo) |
| `ls [path]` | liste un répertoire | `resolve`, profondeur/nb d'entrées bornés |
| `echo <text> > <path>` | écrit/écrase un fichier | `resolve`, taille max |
| `grep <pattern> <path...>` | recherche texte | `resolve`, fichiers bornés en taille |

Tout le reste est refusé (`unknown command`). Pas de pipes, pas de redirections
sauf `>` pour `echo`, pas de variables, pas de sous-commandes, pas de `&&`/`;`.

### Parsing

- Parsing maison, strict, sans shell : la commande est découpée en tokens simples
  (mini-quoteur explicite et testé, style `"..."` et `'...'` comme bash).
- Aucun appel à `sh -c` / `bash -c` pour les commandes whitelistées.

## Commandes pures

- Définies par l'humain dans un fichier dédié du repo (ex. `console/pures.txt`),
  que l'agent ne peut pas modifier (revu en PR, protégé par review humaine).
- Invoquées par l'agent via `pure <nom>` ; la console ne parse pas le contenu,
  elle le passe tel quel à un vrai shell avec `cwd = root` et un environnement
  réduit (variables d'env minimales).
- **Fichiers sensibles** : déclarés par des regex (ex. `.*\.env`, `.*secret.*`,
  `tests/.*`). Toute commande pure qui touche à un fichier matchant est soit
  refusée, soit soumise à un scan renforcé.
- Avant chaque exécution :
  1. **checksum** (sha256) du fichier de commandes pures consigné ;
  2. **scan des changements** depuis la dernière exécution validée (diff du
     fichier + du code de test associé) pour vérifier qu'il n'y a pas de code
     adversarial introduit ;
  3. exécution uniquement si checksum attendu == checksum constaté et scan OK.

## Organisation du code (proposition, à discuter)

```
src/
  lib.rs
  console.rs        // boucle REPL / API d'exécution
  resolve.rs        // resolve() : confinement root, unique point de passage
  commands/         // cat, ls, echo, grep (whitelist)
  pure.rs           // commandes pures : checksum + scan des changements + exécution
console/
  pures.txt         // fichier de commandes pures (humain only)
  sensibles.txt     // regex des fichiers sensibles
tests/
  resolve.rs        // tests d'évasion (.., symlinks, absolus, unicode...)
  commands.rs
```

## Principes

- Fail-closed : au doute, refuser.
- Une seule fonction de résolution de chemins, testée agressivement.
- Aucun `unsafe` dans la v1.
- Pas de dépendances réseau ; deps minimales (`std` + éventuellement `sha2`, `regex`).

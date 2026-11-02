# Fausse console (fake console) — design

Première brique du harness : une console volontairement très limitée et très sûre,
utilisable par un agent sans risque de sortie de son périmètre.

## Objectifs

1. **Périmètre absolu** : tout accès fichier est confiné sous un `root` donné.
   Aucune commande ne doit pouvoir lire/écrire hors de ce root, par quelque moyen
   que ce soit (`..`, symlinks, chemins absolus, encodages, etc.).
2. **Opérations limitées** : seules quelques commandes whitelistées sont disponibles.
3. **Commandes de test "putes"** : commandes arbitraires définies par l'humain dans
   un fichier du repo que l'agent ne peut pas modifier, avec checksum + scan par un
   autre agent avant chaque exécution.

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

| Commande | Effet | Contraintes |
|---|---|---|
| `cat <path>` | lit un fichier | `resolve` + taille max (ex. 1 Mo) |
| `ls [path]` | liste un répertoire | `resolve`, profondeur/nb d'entrées bornés |
| `echo <text> > <path>` | écrit/écrase un fichier | `resolve`, taille max, refuse d'écraser un fichier hors whitelist si besoin |
| `grep <pattern> <path...>` | recherche texte | `resolve`, fichiers bornés en taille |

Tout le reste est refusé (`unknown command`). Pas de pipes, pas de redirections
sauf `>` pour `echo`, pas de variables, pas de sous-commandes, pas de `&&`/`;`.

### Parsing

- Parsing maison, strict, sans shell : la commande est découpée en tokens simples
  (pas d'interprétation de quotes complexes — ou alors un mini-quoteur explicite
  et testé).
- Aucun appel à `sh -c` / `bash -c` pour les commandes whitelistées.

## Commandes "putes" (tests)

- Définies par l'humain dans un fichier dédié du repo (ex. `tests/commands.pwd`),
  que l'agent ne peut pas modifier (revu en PR, protégé par review humaine).
- Avant chaque exécution :
  1. **checksum** du fichier (sha256) consigné ;
  2. **scan par un autre agent** (revue indépendante du contenu) ;
  3. exécution uniquement si checksum attendu == checksum constaté et scan OK.
- Ces commandes s'exécutent via un vrai shell, mais toujours avec `cwd = root`
  et, si possible, dans un environnement réduit (variables d'env minimales).

## Organisation du code (proposition, à discuter)

```
src/
  lib.rs
  console.rs        // boucle REPL / API d'exécution
  resolve.rs        // resolve() : confinement root, unique point de passage
  commands/         // cat, ls, echo, grep (whitelist)
  pute.rs           // commandes de test : checksum + scan + exécution
tests/
  resolve.rs        // tests d'évasion (.., symlinks, absolus, unicode...)
  commands.rs
  commands.pwd      // fichier de commandes "putes" (humain only)
```

## Principes

- Fail-closed : au doute, refuser.
- Une seule fonction de résolution de chemins, testée agressivement.
- Aucun `unsafe` dans la v1.
- Pas de dépendances réseau ; deps minimales (`std` + éventuellement `sha2`).

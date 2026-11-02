# Architecture méta de la-console

La console n'est pas "une boucle qui exécute des commandes" : c'est un
**pipeline de validation composable**, appliqué à trois niveaux imbriqués.
Toute donnée (commande, chemin, sortie) passe par les mêmes gardes, quel que
soit le niveau auquel elle se trouve.

## Vue d'ensemble (diagramme)

```mermaid
flowchart TD
    H[harness] -->|ligne JSON {op: exec, cmd}| C[la-console]
    C --> P[1. VALIDATION ENTRÉE<br/>parse shell-words<br/>expansion variables/globs]
    P --> V[2. BOUCLE DE VALIDATION CHEMINS<br/>resolve() + sensibles + bornes<br/>pour chaque chemin impliqué]
    V -->|ok| X[exécution avec timeout]
    V -->|denied| R1[réponse JSON erreur]
    X --> S[3. VALIDATION SORTIE<br/>troncature, masquage secrets, formatage]
    S -->|flux interne pipe/redirection| X2[commande suivante de la chaîne]
    S -->|sortie finale| R2[réponse JSON {ok, code, stdout, stderr}]
    R2 --> H
    R1 --> H
```

## Les trois niveaux de validation

### Niveau 1 — Validation de l'entrée (la commande elle-même)

- Tokenisation (`shell-words`), puis interprétation cadrée des opérateurs
  autorisés (`&&`/`;`, pipes, redirections, globs `wax`, variables déclarées).
- Résultat : un **arbre de commandes** (pas une chaîne) — chaque nœud est une
  commande whitelist ou trusted, chaque branche passe ensuite individuellement
  par les niveaux 2 et 3.

### Niveau 2 — Boucle de validation des chemins (composable)

La boucle centrale, appliquée **à chaque chemin de chaque nœud de l'arbre** :

1. `resolve(root, input)` → confinement (canonicalize, `..`, symlinks) ;
2. test fichiers sensibles (regex sur le chemin relatif) ;
3. bornes (taille lecture/écriture, nb de matchs de glob, quota root).

C'est la même boucle pour un `cat`, une destination de `>`, un glob扩展é,
un argument de commande trusted. Aucune exception.

### Niveau 3 — Validation de la sortie

Avant qu'une sortie ne quitte la console (ou ne parte dans un pipe) :

- troncature à `max_output_bytes` (début + fin, flag `truncated`) ;
- masquage des secrets connus (token GitHub App → `***`) ;
- formatage : **flux brut** entre commandes internes (pipes/redirections),
  **JSON `{ok, code, stdout, stderr}`** uniquement vers le harness.

## Composition : pourquoi c'est composable

- `a && b` : deux nœuds, chacun passe par les 3 niveaux ; stop au 1er échec.
- `a | b` : la sortie validée (niveau 3, flux brut) de `a` devient l'entrée de
  `b` ; `b` repasse par les 3 niveaux. Whitelist uniquement, jamais trusted.
- `a > f` : `f` est validé par la boucle niveau 2 **avant** l'exécution de `a` ;
  la sortie écrite est aussi bornée.

## Ce qu'il faut retenir

> Toute donnée qui entre, touche au disque, ou sort passe par une validation
> explicite. Il n'y a pas de "chemin rapide".

## Décisions sur les points ouverts

- ✅ **Commandes trusted et niveau 2** : oui, leurs arguments de chemin passent
  par la boucle de validation (resolve + sensibles), **sauf flag explicite**
  déclaré par l'humain dans `trusted.txt` (ex. `trusted run-tests --no-path-check`)
  pour les commandes qui doivent légitimement toucher hors du root.
- **Masquage des secrets** : voir `fake-console.md` section "Secrets" — toute
  occurrence du token (ou autre secret connu du harness) dans `stdout`/`stderr`
  est remplacée par `***` avant envoi de la réponse JSON. À trancher : s'applique
  aussi aux flux internes entre commandes, ou seulement à la sortie vers le harness ?

## Questions ouvertes à discuter

- [ ] Le masquage des secrets s'applique-t-il aussi aux flux internes
      entre commandes (pipes), ou seulement à la sortie finale vers le harness ?

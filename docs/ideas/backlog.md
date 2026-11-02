# Backlog d'idées — à discuter

## ⏳ À trancher (décisions en attente)

- [x] Format des sorties : **JSON structuré** `{ok, code, stdout, stderr}` entre
      console et harness (séparation métadonnées/contenu, parsing fiable). Le harness
      peut présenter le `stdout` brut à l'agent

## ✅ Décidés

- [x] Syntaxe des commandes whitelist proche du shell bash (les agents connaissent
      bien bash, c'est mieux pour l'adoption du tool)
- [x] Commandes pures invoquées par nom (`pure <nom>`), non interprétées par la
      console, passées telles quelles au shell, considérées sûres car validées
      par l'humain
- [x] Console **stateless** : chaque commande repart du root, pas de `cd`
      persistant (moins d'état à valider, chaque `resolve()` repart d'une base connue)
- [x] Interface : **sous-processus dédié**, protocole stdin/stdout ligne par ligne
      (isolation par processus — un crash de la console ne fait pas planter le harness,
      ex. lecture d'un très gros fichier)
- [x] Confinement : **v1 logique pur Rust** (resolve + canonicalize + tests d'évasion
      CI, sans privilèges, portable) ; **v2 sur le serveur remote : user namespace
      non privilégié + chroot** par-dessus (defense in depth). Jamais de root global
      au lancement de l'agent. Vérifier `unprivileged_userns_clone` au boot du serveur
- [x] Fichiers sensibles déclarés par regex ; scan des changements (diff) entre
      chaque exécution pour détecter du code adversarial

## Sécurité / confinement

- [ ] Vecteurs d'évasion prioritaires à tester : `..`, symlinks (créés avant/après
      canonicalize), chemins absolus, unicode, trailing slashes, `//`, NUL bytes
- [ ] Bornes : profondeur max de `ls`, nb max d'entrées, taille max par fichier
- [ ] `echo >` écrase-t-il un fichier existant ? et `>>` (append) un jour ?
- [ ] Hardlinks : détectables portablement en Rust stable ?
- [ ] Timeout par commande ?

## Commandes whitelist

- [ ] Ajouter `head`/`tail` ? `wc` ? `mkdir` ? (`rm` probablement non)

## Harness (brique suivante)

- [ ] Comment la console s'intègre au harness : subprocess ? canal de
      communication (stdin/stdout, socket locale, fichiers) ?

## Process

- [ ] Revue humaine obligatoire sur toute modification de `console/pures.txt`
      et `console/sensibles.txt` (branch protection / CODEOWNERS)
- [ ] Tests d'évasion en CI obligatoires avant merge

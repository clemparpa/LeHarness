# Backlog d'idées — à discuter

## ⏳ À trancher (décisions en attente)

- [x] **Passage des secrets (GitHub App token) — ✅ décidé (option A)** : le token
      reste **en mémoire dans le harness**, transite uniquement par **fd / socket
      unix** vers un **credential helper git externe, hors du root et non lisible
      par l'agent**. Jamais dans l'env du shell (`printenv` ne voit rien), jamais
      en argv (`ps` ne voit rien), jamais dans un fichier lisible par l'agent.
      `HOME` du sous-processus pointé hors du root avec une config git propre ;
      `.git/config` et `.git-credentials` déclarés fichiers sensibles. Le token
      est aussi **masqué** (`***`) dans stdout/stderr des réponses JSON

## ✅ Décidés

## ✅ Décidés

- [x] Syntaxe des commandes whitelist proche du shell bash (les agents connaissent
      bien bash, c'est mieux pour l'adoption du tool)
- [x] Commandes trusted invoquées par nom (`trusted <nom>`), non interprétées par la
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

## Module navigateur (v2, hors la-console — option B : process/conteneur dédié, réseau localhost uniquement)

Candidats à évaluer (que du Rust/Zig, pas de C++/Swift) :

| Projet | Langage | Empreinte | Pilotage | Notes |
|---|---|---|---|---|
| [oxibrowser](https://github.com/project-oxi/oxibrowser) | Rust pur (Servo) | ~24-44 Mo binaire, ~8 Mo RAM | Compatible CDP (Puppeteer/Playwright) | Conçu pour les agents IA, SSRF protection intégrée (blocage CIDR), robots.txt |
| [Lightpanda](https://lightpanda.io/) | Zig | 9x moins de RAM que Chrome, 11x plus rapide | Compatible CDP | Headless, open source, pensé scraping/automation IA |
| [Servo](https://servo.org/) | Rust | Léger | Embarquable en librairie | Moteur officiel, mais historiquement instable en embarqué (crashs rapportés) |
| [Boa](https://github.com/boa-dev/boa) | Rust | Très léger | Librairie embarquable | Moteur JS seul, pas de DOM/CSS — insuffisant seul pour rendre une page |

- [ ] Choix du moteur à trancher plus tard après évaluation concrète (stabilité, couverture sites modernes)
- [ ] Quel que soit le moteur : process/conteneur isolé, réseau restreint à localhost, rlimits stricts, sorties (screenshots/DOM) passent par `resolve()` + bornes

## Harness (brique suivante)

- [x] **Hooks de session** (décidé en principe) : le harness expose des hooks
      (démarrage / checkpoint / fin / reprise de session) ; la persistance elle-même
      (DB managée, stockage du root, verrou anti-parallèle) est gérée par la couche
      au-dessus (orchestrateur GitHub App, jobs éphémères), pas par le harness

- [ ] Comment la console s'intègre au harness : subprocess ? canal de
      communication (stdin/stdout, socket locale, fichiers) ?

## Process

- [ ] Revue humaine obligatoire sur toute modification de `console/trusted.txt`
      et `console/sensibles.txt` (branch protection / CODEOWNERS)
- [ ] Tests d'évasion en CI obligatoires avant merge

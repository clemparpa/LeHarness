# Backlog d'idées — à discuter

> Note : la spécification de la-console vit dans le repo `clemparpa/LaConsole`
> (dossier `spec/`), pas ici. Ce dépôt ne contient que la doc du harness.

## ⏳ À trancher (décisions en attente)

- [x] **Passage des secrets (GitHub App token) — ✅ décidé (option A)** : le token
      reste **en mémoire dans le harness**, transite uniquement par **fd / socket
      unix** vers un **credential helper git externe, hors du root et non lisible
      par l'agent**. Jamais dans l'env du shell (`printenv` ne voit rien), jamais
      en argv (`ps` ne voit rien), jamais dans un fichier lisible par l'agent.
      `HOME` du sous-processus pointé hors du root avec une config git propre ;
      `.git/config` et `.git-credentials` déclarés fichiers sensibles. Le token
      est aussi **masqué** (`***`) dans stdout/stderr des réponses JSON
- [x] **Rôle de broker de credentials ✅ décidé** : le harness joue le rôle de
      **reverse proxy local** auprès des outils (pattern « phantom token »,
      inspiré de nono) — notamment avec la-console. L'outil ne reçoit qu'un
      token fantôme ; le vrai secret reste côté harness et n'est injecté qu'à
      la sortie, vers la destination autorisée uniquement.
- [x] **Orchestration multi-sessions ✅ goal prioritaire (étape 8, ré-priorisée)** :
      le serveur traite plusieurs sessions en même temps, **un processus par
      session** via le lanceur ligne-JSON, thread lecteur par session → `mpsc`.
      Bornes : `--max-sessions`, flock par repo, rlimits, timeout par message, audit
      par session ; l'orchestrateur ne détient aucun secret. Le GitHub App remote
      (webhooks) reste en v2 par-dessus, toujours dépriorisé.
- [x] **Appels d'outils en parallèle ✅ goal prioritaire (étape 9, avec la TUI)** :
      `submit` non bloquant + worker threads bornées (`--max-parallel-tools`,
      défaut 4, plafond 16), résultats taggés par id ; requêtes avec redirection
      (`>`/`>>`) sérialisées par mutex ; timeout + kill + masquage par appel.
- [x] **Concurrence bornée, sans runtime async ✅ décidé (révisé nov. 2026)** :
      « synchrone » = pas de runtime async, pas « pas de concurrence ». La
      concurrence (multi-sessions, outils parallèles) passe par processus/threads
      OS bornés : unités lourdes (attente bloquante inhérente), isolation par
      processus (SecretStore + phantom token par session), déterminisme/audit par
      unité, zéro dépendance ajoutée (pas de `tokio` dans le `Cargo.lock`).
      Runtime async envisageable uniquement sur besoin mesuré (C10k, fan-out HTTP
      massif), isolé dans une couche réseau, jamais dans le cœur.
      Détail : `spec/ecosystem.md` §8.
- [x] **Stack TUI (étape 9) ✅ décidé** : `ratatui` + `crossterm` (boucle
      `poll`/`read` + thread d'événements → `mpsc`, rendu sur stderr), `color-eyre`
      + `signal-hook` pour la restauration du terminal. `tui-rs` archivé,
      `cursive`/`termion`/`termwiz`/`blessed-rs` non retenus, `tuirealm` reporté.
      Tour d'horizon complet : `spec/ecosystem.md`.
- [x] **ACP en v1 : rôle `Agent` maison sans runtime async ✅ décidé** : la spec
      ACP v1 est stable (oct. 2026) mais le SDK Rust officiel est async
      (futures/tokio) — en v1 on implémente le rôle `Agent` au-dessus de notre
      protocole ligne-JSON ; le SDK officiel reste l'option si besoin mesuré.

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

## Conteneurs (hors la-console — à trancher plus tard)

Jamais de `docker` / socket docker.sock (équivalent root hôte). Pistes notées,
le choix sera fait en temps voulu :

- **Podman rootless (daemonless)** — piste privilégiée pour la v1 : pas de démon,
  pas de socket à protéger (contrairement à dockerd) ; chaque `podman run` est un
  sous-processus spawn/kill par la console, avec `--network none`, rlimits,
  `--read-only`, allowlist d'images gérée côté console (jamais de pull arbitraire).
  Plus haut niveau et plus simple à utiliser que les runtimes bruts.
- **Runtimes OCI one-shot (pas de démon)** : [runc](https://github.com/opencontainers/runc),
  [crun](https://github.com/containers/crun), [youki](https://github.com/containers/youki)
  (en Rust). Invoqués directement, un process par conteneur. **crun** : écrit en C,
  démarrage 2-3x plus rapide que runc, faible empreinte, runtime par défaut de
  podman — mais CLI bas niveau : il faut construire soi-même le bundle OCI
  (`config.json` + rootfs), ce qui donne un contrôle total et une surface minimale.
  Piste pour quand on voudra orchestrer nous-mêmes ; podman reste plus simple en v1.
- **Couche forte en v2 (defense in depth, déploiement remote)** : micro-VM
  [Firecracker](https://firecracker-microvm.github.io/) / Kata, ou
  [gVisor](https://gvisor.dev/) (runtime OCI `runsc`, interception des appels
  système, portable sans KVM). Pour contrer une 0-day noyau dans un conteneur.

Règles quel que soit le choix : allowlist d'images déclarée par l'humain,
réseau none/localhost par défaut, aucun montage de volume du host (ou répertoire
validé via `resolve()` uniquement), rlimits stricts, timeout, kill systématique,
sorties bornées + masquées.

## Harness (cœur — en cours, voir ROADMAP.md)

- [x] **Hooks de session** (décidé) : le harness expose des hooks
      (démarrage / checkpoint / fin / reprise de session) ; la persistance elle-même
      (DB managée, stockage du root, verrou anti-parallèle) est gérée par la couche
      au-dessus (orchestrateur GitHub App, jobs éphémères), pas par le harness
- [x] **Indépendance harness / console** (décidé) : deux repos séparés, aucune
      dépendance de code ; la console n'est qu'un outil optionnel branché comme
      binaire externe (étape 7 de `ROADMAP.md`)
- [x] **Intégration console → harness** (décidé) : via le lanceur générique de
      sous-processus du harness (protocole ligne JSON sur stdin/stdout, étape 3),
      jamais en librairie dans le processus du harness
- [x] **Local-first** (décidé) : le harness tourne en local sur un repo local ;
      le mode GitHub App remote s'ajoute par-dessus sans toucher au cœur

## Process

- [ ] Revue humaine obligatoire sur toute modification de `console/trusted.txt`
      et `console/sensibles.txt` (branch protection / CODEOWNERS)
- [ ] Tests d'évasion en CI obligatoires avant merge

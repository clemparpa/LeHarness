# Sécurité du harness

## Modèle de menaces

Le harness est le composant qui **détient les secrets** (token GitHub App, clés
API de l'agent). Les agents et les outils qu'il lance sont considérés comme
**non fiables**. Conséquence : aucun secret ne doit jamais atteindre un
sous-processus lancé, un fichier, ou une sortie non masquée.

## Règles absolues

- **Secrets en mémoire uniquement** : chargés au lancement (env ou fichier de
  config local du user), jamais écrits sur disque par le harness.
- **Jamais dans l'env ou l'argv des sous-processus** (`printenv` / `ps` ne
  voient rien). Si un outil a besoin d'un credential, il passe par fd / socket
  unix vers un helper externe placé hors du root (mécanisme prévu côté
  la-console, à réutiliser).
- **Masquage `***`** de toute occurrence d'un secret connu dans toute sortie
  transmise à l'extérieur (réponses des outils, logs).
- **Sous-processus uniquement** pour les outils : jamais de code d'outil dans
  le processus du harness. Un crash d'un outil ne tue pas le harness.
- **Fail-closed** : réponse mal formée d'un outil = erreur, pas d'interprétation
  partielle.
- Aucun `unsafe` en v1.

## Local vs remote

- **Local** : le harness tourne avec les droits du user, sur un repo local.
  Aucune confiance réseau requise.
- **Remote (GitHub App)** : déploiement serveur en v2 avec user namespace non
  privilégié + chroot par-dessus (defense in depth), jamais de root global au
  lancement. Vérifier `unprivileged_userns_clone` au boot du serveur.

## Politique déclarée par le harness

- `trusted.txt` et `sensitives.txt` vivent dans **le repo de l'agent**, lus par
  le harness au démarrage de la session et transmis aux outils qui les
  appliquent (la console les applique via `--trusted-file` / `--sensitives-file`).
- Le harness déclare la politique ; les outils l'appliquent. La responsabilité
  de l'exécution (validation, confinement, limites, kill) reste côté outil,
  jamais déléguée au harness ni à l'agent.

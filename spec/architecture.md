# Architecture du harness

Le harness n'est pas « une boucle qui exécute des commandes » : c'est un **cœur
de session** entouré de couches de transport optionnelles. La console
(`la-console`, repo séparé) n'est qu'un outil parmi d'autres, branché comme
binaire externe.

## Vue d'ensemble (diagramme)

```mermaid
flowchart TD
    T1[CLI local] --> H
    T2[ACP - éditeur] --> H
    T3[Orchestrateur GitHub App - dépriorisé] --> H
    T4[TUI Rust de base - prioritaire] --> H
    H[harness - coeur de session<br/>hooks: start / checkpoint / end / resume] --> S[SecretStore<br/>secrets en memoire, masquage ***]
    H --> P[Persistance<br/>trait injecte par la couche appelante]
    H --> L[Lanceur de sous-processus<br/>protocole ligne JSON, timeout, kill]
    L --> O1[outil externe<br/>ex. la-console]
    L --> O2[autre outil...]
```

## Le cœur (maison)

### Session + hooks

- `Session` : id, root du repo de l'agent, état courant.
- Cycle de vie émis via des hooks : `on_start`, `on_checkpoint`, `on_end`,
  `on_resume`.
- La **persistance est un trait injecté** par la couche appelante (CLI local,
  orchestrateur) : le cœur n'ouvre jamais de fichier lui-même.

### SecretStore

- Secrets chargés au lancement (env ou fichier de config local), gardés **en
  mémoire uniquement**, avec refresh automatique quand ils expirent.
- Toute sortie transmise à l'extérieur passe par le masquage (`***`).

### Lanceur de sous-processus

- Spawn d'un binaire externe, protocole **une ligne JSON en entrée, une ligne
  JSON en sortie**, timeout, kill propre du fils à la fin.
- Réponses mal formées = rejet fail-closed.
- Le stdin/stdout du fils appartiennent au protocole ; les secrets ne sont
  **jamais** passés par env ou argv.

### Proxy de credentials (broker)

- Le harness peut jouer le rôle de **reverse proxy local** pour les outils :
  l'outil ne reçoit qu'un *phantom token* de session ; le vrai secret, détenu
  uniquement par le harness, est injecté à la sortie vers la destination
  autorisée uniquement (pattern « phantom token », inspiré de nono).
- Concerne notamment la-console et tout outil ayant besoin d'un credential réseau.

## Couches de transport optionnelles

Aucune n'est obligatoire ; le cœur fonctionne sans.

| Couche | Rôle | Tech |
|---|---|---|
| CLI local | `le-harness run --repo <chemin>`, boucle interactive | clap |
| TUI Rust de base | interface terminal standard au-dessus du cœur (prioritaire) | `ratatui` + `crossterm`, boucle synchrone (voir [ecosystem.md](ecosystem.md)) |
| ACP | piloté comme agent depuis un éditeur (Zed...) | v1 : rôle `Agent` maison synchrone au-dessus du protocole ligne-JSON ; SDK officiel `agent-client-protocol` (async) si besoin mesuré |
| Orchestrateur GitHub App | webhooks, jobs éphémères (remote, dépriorisé) | reqwest + rustls, mode blocking, côté harness |

> **Synchrone par défaut** : aucune couche n'embarque de runtime async en v1 ;
> critères de réouverture dans [ecosystem.md](ecosystem.md) §8.

## Règle d'indépendance

> Aucun `use la_console` nulle part. La console est invoquée comme **binaire
> externe** via le lanceur générique, jamais comme dépendance de code.

# Étape 1 — Cœur : session + hooks

## Objectif
Un cœur de session maison (`crates/harness/src/session.rs`) avec cycle de vie
explicite (démarrage / checkpoint / fin / reprise) émis via des hooks, et une
persistance **injectée par trait** : le cœur n'ouvre jamais de fichier lui-même.

## Contrat / API
- `SessionId` (newtype `String`, `SessionId::generate()` sans dépendance externe),
  `SessionState` (`Running` / `Ended`), `Snapshot` (id, root du repo, état,
  compteur de checkpoints).
- `Session` : `id()`, `root()`, `state()`, `checkpoints()`, `snapshot()` ;
  constructeur `Session::start(id, root, hooks, store)` et méthodes
  `checkpoint()`, `end()`, `Session::resume(id, root, hooks, store)`.
- Trait `Hooks` : `on_start` / `on_checkpoint` / `on_end` / `on_resume`, chacun
  recevant `&Snapshot` (impls par défaut vides).
- Trait `Persistence` injecté par la couche appelante, **indépendant du backend**
  (fichiers JSON — étape 5, `redb`, Postgres, …) :
  `save(&self, &Snapshot) -> Result<(), PersistenceError>` et
  `load(&self, &SessionId) -> Result<Option<Snapshot>, PersistenceError>`.
  Contrat : `save` fait l'*upsert* du dernier snapshot d'une session (clé = `Snapshot::id`) ;
  `load` renvoie le snapshot le plus récent pour cet id, ou `None`.
  Méthodes en `&self` pour s'adapter aux pools de connexions (style sqlx) ;
  v1 synchrone de bout en bout (position « rester synchrone » reprise de la-console).
- Machine à états fail-closed : `checkpoint`/`end` sur session terminée = erreur ;
  `resume` d'un id inconnu = `NotFound` ; `resume` d'une session terminée =
  `AlreadyEnded` ; `resume` avec une racine différente de celle persistée =
  `RootMismatch` ; échec de persistance à `start` = la session ne démarre pas
  et aucun hook n'est émis.
- Erreurs typées (`SessionError`, `PersistenceError`) via `thiserror`.
- Ordre des opérations : persistance d'abord, hook ensuite (un échec de
  persistance bloque le cycle de vie avant toute notification).

## Testable
Tests unitaires du cycle de vie dans `session.rs` avec un double de persistance
en mémoire (`Vec<Snapshot>`) et un double de hooks qui enregistre les événements :
démarrage, checkpoints successifs, fin, double fin refusée, checkpoint après fin
refusé, reprise (état `Running`, hook `on_resume`), reprise d'id inconnu,
reprise de session terminée, racine différente à la reprise, échec de
persistance, unicité des `SessionId::generate()`.

## Hors scope
Persistance fichier / `redb` (étape 5), CLI `run --repo` (étape 4),
`SecretStore` (étape 2), lanceur de sous-processus (étape 3), transports
(ACP étape 6, orchestrateur étape 8).

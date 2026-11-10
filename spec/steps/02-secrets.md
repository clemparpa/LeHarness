# Étape 2 — Gestion des secrets (maison)

## Objectif
Un `SecretStore` maison (`crates/harness/src/secrets.rs`) qui garde les secrets
**en mémoire uniquement** : chargés au lancement (variable d'env ou fichier de
config local du user, en lecture seule), jamais écrits sur disque par le harness,
jamais transmis à l'env/argv des sous-processus (règle appliquée côté lanceur,
étape 3), et **masqués `***`** dans toute sortie transmise à l'extérieur.

## Contrat / API
- `SecretStore` : conteneur `name -> { value, expires_at? }` en mémoire.
  - `SecretStore::new()` / `Default` (store vide) ;
  - `SecretStore::from_env(names: &[&str]) -> Result<Self, SecretError>` : charge
    chaque variable d'env nommée ; **fail-closed** — une variable absente =
    erreur `EnvVarMissing`, pas un store partiel ;
  - `SecretStore::from_file(path) -> Result<Self, SecretError>` : lit un fichier
    de config local au format `NAME=value` par ligne (lignes vides et
    commentaires `#` ignorés), **en lecture seule** — le harness n'écrit jamais
    ce fichier ; ligne mal formée = erreur `MalformedLine` ;
  - `insert(name, value, ttl: Option<Duration>)` : upsert d'un secret avec
    expiration optionnelle (refresh = ré-`insert` avec une nouvelle valeur/TTL,
    ex. token GitHub App à validité 1 h) ;
  - `get(&self, name) -> Option<&str>` : `None` si absent **ou expiré** ;
  - `is_expired(&self, name) -> bool`, `remove(&mut self, name) -> bool`,
    `len()`, `is_empty()` ;
  - `mask(&self, text: &str) -> String` : remplace **toute occurrence** de chaque
    valeur de secret connue (y compris les secrets expirés — une ancienne valeur
    a pu fuiter dans des logs) par `***`. Les valeurs vides ne sont jamais
    masquées (sinon tout le texte serait remplacé) ; les secrets les plus longs
    sont remplacés en premier (un secret ne doit pas tronquer un autre secret
    qui le contient).
- Erreurs typées `SecretError` via `thiserror` (`EnvVarMissing`, `ReadFailed`,
  `MalformedLine`).
- Le store ne touche jamais à l'environnement des processus fils : aucun secret
  n'est exposé via env/argv (ce sera câblé côté lanceur de l'étape 3).

## Testable
- Tests unitaires dans `secrets.rs` : insert/get, expiration (TTL écoulé ->
  `get` renvoie `None`), refresh par ré-insert, `remove`, `from_env` (ok + variable
  manquante = erreur fail-closed), `from_file` via `tempfile` (lignes valides,
  commentaires, ligne mal formée), masquage (occurrence simple, occurrences
  multiples, secret présent dans un autre, secret vide ignoré, secret expiré
  toujours masqué).
- **proptest** « un secret ne fuit jamais dans une sortie masquée » : pour tout
  secret non vide et tout texte arbitraires, la sortie masquée ne contient pas
  le secret.

## Hors scope
Transmission des secrets aux outils (jamais par env/argv — étape 3), persistance
sur disque (interdite), `zeroize` en v1 (hors deps figées), secrets via fd/socket
unix (mécanisme côté la-console, réutilisé plus tard), orchestrateur GitHub App
(étape 8).

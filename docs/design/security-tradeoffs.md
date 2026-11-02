# Compromis de sécurité — où placer le curseur ?

Analyse des implications de sécurité des choix encore ouverts, pour décider
en connaissance de cause. Chaque axe est présenté comme un curseur entre
"max sécurité / min commodité" et "max commodité / min sécurité".

## Axe 1 — Interface : même processus vs processus séparé

| Option | Ce que ça implique côté sécurité |
|---|---|
| **Librairie Rust dans le processus du harness** | La console partage la mémoire et les privilèges du harness. Une faille dans `resolve()` ou le parser = compromission directe du harness (et de tout ce qu'il peut toucher). Pas d'isolation OS. Debuggage et tests faciles, perfs max. |
| **Sous-processus dédié** | Isolation par processus : un crash ou une évasion de la console ne tue pas le harness et hérite seulement des droits du user qui le lance. Coût : protocole de communication à définir (stdin/stdout, socket locale, fichiers), plus de surface d'attaque sur ce protocole. |
| **Sous-processus + sandbox OS** (voir axe 2) | Même idée, poussée plus loin. |

**Curseur recommandé** : sous-processus au minimum, même en v1. Le harness est le composant qui détient les secrets (clés API de l'agent), donc la console ne devrait jamais tourner dans son espace mémoire.

## Axe 2 — Niveau de confinement du root

| Option | Ce que ça protège | Ce que ça ne protège pas / coût |
|---|---|---|
| **Logique pur Rust (`resolve()` + canonicalize)** | `..`, chemins absolus, symlinks résolus, hardlinks vérifiés. Suffisant contre un agent "honnête mais maladroit" et contre la plupart des évasions naïves. Zéro privilège requis, portable, testable. | Ne protège pas contre : symlink créé *après* la canonicalisation (race TOCTOU), bugs dans `resolve()` lui-même, attaques sur le noyau/les FS exotiques, lecture de métadonnées hors root. Un bug = évasion totale. |
| **+ chroot / user namespace (Linux)** | Le noyau bloque réellement l'accès hors root, même en cas de bug applicatif. Defense in depth. | Nécessite des privilèges (root ou `CAP_SYS_CHROOT` / userns), moins portable (Windows/macOS différents), setup plus lourd. |
| **Conteneur dédié** | Isolation maximale (réseau, FS, processus). | Hors scope v1 probablement, orchestration lourde. |

**Curseur recommandé** : v1 en logique pur Rust (avec tests d'évasion agressifs), mais architecturer pour pouvoir ajouter un userns/chroot plus tard sans changer l'API. Le confinement logique reste la **première** ligne, jamais la seule quand on montera en maturité.

## Axe 3 — Session (cwd relatif persistant) vs stateless

| Option | Implication sécurité |
|---|---|
| **Stateless** (chaque commande repart du root) | Moins d'état à gérer, moins de risque d"'oublier" qu'on est dans un sous-dossier ; chaque `resolve()` repart d'une base connue. Plus verbeux pour l'agent. |
| **Session avec `cd`** | Plus proche de bash (les agents s'y attendent). Mais il faut que `cd` lui-même passe par `resolve()`, et chaque commande dépend de l'état courant → plus de surface de bug (ex. : état corrompu après une commande refusée). |

**Note** : si on choisit session, `cd` devient une commande whitelistée de plus, avec les mêmes contraintes `resolve()`.

## Axe 4 — Format des sorties (texte vs JSON)

Peu d'impact sécurité direct, mais un point subtil : **si la sortie est du texte brut qui peut contenir n'importe quoi (contenu de fichiers lus), le harness/agent qui la parse peut être induit en erreur** (ex. : un fichier contenant `"commande suivante"`). Un format structuré (JSON avec champs `stdout`/`stderr`/`code`) rend la séparation contenu/métadonnées explicite et réduit les risques d'injection dans l interprétation de la sortie.

**Curseur recommandé** : JSON structuré pour le transport console ↔ harness, même si l'agent voit au final du texte lisible.

## Axe 5 — Commandes pures : exécution

Déjà décidé (passage direct au shell), mais le curseur restant est **l'environnement d'exécution** :

- `cwd = root` forcé ✅
- variables d'env réduites (pas de `PATH` hérité, pas de secrets du harness dans l'env) — à confirmer
- timeout par commande pure — à confirmer
- user OS dédié (pas le même user que le harness) — le plus sûr, coût : setup

## Synthèse — proposition de curseur v1

| Axe | Curseur v1 proposé |
|---|---|
| Interface | Sous-processus dédié, protocole JSON sur stdin/stdout |
| Confinement | Logique pur Rust, tests d'évasion en CI, archi prête pour userns plus tard |
| Session | Stateless (v1), `cd` en option v2 si besoin |
| Sorties | JSON structuré (stdout/stderr/code) |
| Pures | shell réel, cwd=root, env minimale, timeout, scan des diffs avant exécution |

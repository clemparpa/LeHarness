# Backlog d'idées à discuter

## Sécurité / confinement

- [ ] V1 : `resolve()` avec canonicalize + rejet `..` + symlinks. Quels vecteurs
      d'évasion tester en priorité ? (unicode, trailing slashes, `//`, chemins
      Windows si pertinent, NUL bytes)
- [ ] Profondeur max de `ls`, nombre max d'entrées, taille max par fichier lu
- [ ] Faut-il interdire l'écrasement de fichiers existants avec `echo >` ?
- [ ] Hardlinks : détectables portablement en Rust stable ?
- [ ] Faut-il un timeout par commande ?

## Commandes whitelist

- [ ] Ajouter `head`/`tail` ? `wc` ? `mkdir` ? `rm` (probablement non) ?
- [ ] `echo >> file` (append) ou uniquement écrasement ?
- [ ] Format de sortie : texte brut ou structuré (JSON) pour faciliter le harness ?

## Commandes "putes"

- [ ] Format du fichier de commandes (une par ligne ? nom + commande ?)
- [ ] Où stocker le checksum attendu : dans le repo (signé) ou généré à la volée ?
- [ ] Mécanisme concret du "scan par un autre agent" : même agent, prompt différent ?
      Agent séparé ? Critères de scan formalisés (liste de patterns interdits) ?
- [ ] Environnement d'exécution des commandes pute : variables d'env, PATH réduit,
      `cwd` forcé au root

## Harness (brique suivante)

- [ ] Comment la fausse console s'intègre au harness : API synchrone ? subprocess ?
      Canal de communication (stdin/stdout, socket locale, fichiers) ?
- [ ] Gestion des sessions : état persistant (cwd relatif) ou stateless ?

## Process

- [ ] Revue humaine obligatoire sur toute modification de `tests/commands.pwd`
      (branch protection / CODEOWNERS)
- [ ] Tests d'évasion en CI obligatoires avant merge

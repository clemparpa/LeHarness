# AGENTS.md — instructions pour les agents travaillant sur ce dépôt

## Début de session

Avant tout travail, lire la documentation du projet, dans cet ordre :

- `README.md` — vue d'ensemble du harness ;
- `ROADMAP.md` — feuille de route, identifier l'étape en cours ;
- `spec/README.md` puis les documents transverses de `spec/` (`architecture.md`,
  `security.md`, `testing.md`, `structure.md`) ;
- `spec/steps/NN-*.md` — la spec de l'étape en cours (créée/mise à jour à chaque étape) ;
- `docs/` — réflexions de design et backlog (`docs/ideas/backlog.md`) si le sujet s'y rapporte.

## Règles de travail

- Une étape de `ROADMAP.md` = une PR indépendante, **testable et avec CI verte avant merge**.
- TDD systématique (voir `spec/testing.md`) ; aucun `unsafe` (`#![forbid(unsafe_code)]`).
- Le harness est **indépendant de la-console** : jamais de dépendance de code vers
  `la-console` (repo séparé `clemparpa/LaConsole`) ; la console n'est branché que
  comme binaire externe via le lanceur de sous-processus.
- Ne jamais recopier la spec de la-console dans ce dépôt : elle vit dans `clemparpa/LaConsole`.

## Avant chaque PR

- Mettre à jour la documentation pour qu'elle reflète les changements de la PR :
  `ROADMAP.md` (statut de l'étape), `spec/` (transverses + `spec/steps/NN-*.md`),
  `docs/` si pertinent.
- Remplir la spec de l'étape (`spec/steps/NN-slug.md`) : objectif, contrat/API,
  testable, hors scope.

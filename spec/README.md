# Spécification de LeHarness

**LeHarness** est un harness d'agents IA écrit en Rust : il pilote des sessions
d'agent, détient les secrets en mémoire, et lance des outils externes en
sous-processus confinés. Il tourne **en local** (repo local) comme **en remote**
(piloté par un orchestrateur GitHub App).

Il est **entièrement indépendant de la-console** : la console (repo séparé
`clemparpa/LaConsole`) n'est qu'un outil optionnel branché comme binaire externe.

## Objectifs

1. **Sessions fiables** : cycle de vie explicite (démarrage / checkpoint / fin /
   reprise) via des hooks, persistance déléguée à la couche appelante.
2. **Secrets jamais exposés** : en mémoire côté harness uniquement, jamais sur
   disque, jamais dans l'env/argv des sous-processus, masqués dans les sorties.
3. **Outils en sous-processus** : protocole ligne JSON par ligne, timeout, kill
   propre, fail-closed.
4. **Fail-closed** : au doute, refuser. Aucun « chemin rapide ».

## Principes directeurs

- Cœur sécurité écrit **maison**, volontairement (pas de framework d'agents).
- Aucun `unsafe` en v1 (`#![forbid(unsafe_code)]`).
- Une étape de la roadmap = une PR indépendante, testable, CI verte avant merge.
- Itération par étape : chaque étape a sa spec dédiée dans `spec/steps/NN-*.md`,
  référencée depuis `ROADMAP.md`.

## Sommaire

| Document | Contenu |
|---|---|
| [architecture.md](architecture.md) | Cœur du harness : session + hooks, secrets, lanceur de sous-processus, couches de transport optionnelles |
| [security.md](security.md) | Modèle de menaces, règles des secrets, sous-processus, local vs remote |
| [testing.md](testing.md) | Stratégie de test reprise de la-console : TDD, proptest, e2e, fuzz, mutants, CI |
| [structure.md](structure.md) | Structure du dépôt et dépendances figées |
| [steps/](steps/README.md) | Spec détaillée de chaque étape de la roadmap |

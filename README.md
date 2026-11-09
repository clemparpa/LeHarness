# LeHarness

Harness d'agents en Rust. Il pilote des sessions d'agent (hooks démarrage /
checkpoint / fin / reprise), détient les secrets en mémoire uniquement, et lance
des outils externes en sous-processus confinés (protocole ligne JSON). Il tourne
**en local** (repo local) comme **en remote** (orchestrateur GitHub App).

**la-console** (repo séparé `clemparpa/LaConsole`) n'est qu'un outil optionnel
du harness, branché comme binaire externe : aucune dépendance de code entre les
deux repos.

- [ROADMAP.md](ROADMAP.md) — feuille de route : une PR indépendante et testable par étape
- [spec/](spec/README.md) — spécification du harness (architecture, sécurité, tests, structure, specs d'étapes)
- [docs/](docs/design/fake-console.md) — réflexions de design (incluant la fausse console)

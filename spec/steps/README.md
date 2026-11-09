# Specs des étapes de la roadmap

Chaque étape de `ROADMAP.md` a sa spec dédiée ici : `NN-slug.md` (NN = numéro
d'étape, zéro-paddé). Une étape = une PR indépendante, testable, CI verte avant
merge.

Template d'une spec d'étape :

```markdown
# Étape NN — <titre>

## Objectif
<une phrase>

## Contrat / API
<types, signatures, protocole — ce que la PR doit livrer>

## Testable
<les tests qui prouvent que l'étape est finie>

## Hors scope
<ce qui est reporté à une étape suivante>
```

Voir [00-skeleton.md](00-skeleton.md) pour la première étape.

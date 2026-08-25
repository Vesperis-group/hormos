# ADR 0002 — Compose comme driver séparé

- **Statut** : accepté
- **Date** : 2026-08
- **Contexte** : bootstrap Hormos (Compose non implémenté)

## Contexte

Docker Compose est un format riche et évolutif. Le réimplémenter serait coûteux,
source de divergences de comportement et de failles (parsing, résolution de
variables, montages).

## Décision

Compose sera un **driver séparé** qui **invoque `docker compose`** en
sous-processus. Hormos ne réimplémente **pas** le format Compose et délègue au
binaire officiel.

Contraintes de sécurité :

- Invocation en **tableau d'arguments** (argv), **jamais** via `sh -c` ni
  concaténation de chaînes → pas d'injection de commande.
- Le driver est **isolé** du reste du cœur derrière une interface dédiée.
- Les fichiers Compose fournis par l'utilisateur sont traités comme des entrées
  potentiellement **malveillantes** (montages, volumes, options privilégiées) :
  validation et avertissements sur les options à risque (voir
  [threat-model.md](../threat-model.md)).

## Conséquences

- **+** Compatibilité totale avec Compose, maintenance quasi nulle du format.
- **+** Surface d'attaque réduite (pas de parseur maison à fuzzer dans l'immédiat).
- **−** Dépendance au binaire `docker compose` présent sur l'hôte.
- **−** Contrôle plus grossier qu'une intégration API native (acceptable).

## Alternatives écartées

- **Réimplémenter Compose** : effort disproportionné, risque de divergence et de
  bugs de sécurité.
- **Bibliothèque tierce de parsing Compose** : surface de dépendance et de
  maintenance non justifiée à ce stade.

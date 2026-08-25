# ADR 0001 — Abstraction du runtime conteneur

- **Statut** : accepté — **implémenté**
- **Date** : 2026-08
- **Contexte** : Docker foundation

## Contexte

Hormos doit piloter des conteneurs. Docker est la cible initiale, mais lier
directement le cœur à l'API Docker figerait le projet sur un seul moteur et
rendrait les tests dépendants d'un démon Docker réel.

## Décision

Le moteur de conteneurs est masqué derrière un **trait `ContainerRuntime`**. Le
cœur (`hormos-core`) et les interfaces ne dépendent que de ce trait, jamais
directement d'un client Docker.

- Première implémentation : **Docker via Bollard** (`hormos-docker`).
- Implémentations futures possibles sans toucher aux interfaces : un faux
  déterministe (tests hermétiques, sans démon) et éventuellement **Podman**.

Le trait est **object-safe** : le cœur manipule un `Arc<dyn ContainerRuntime>`.
Ses méthodes étant asynchrones, `#[async_trait]` est utilisé — il produit
exactement la désucrarisation `Pin<Box<dyn Future>>` qu'il faudrait écrire à la
main pour rendre un trait `async` utilisable derrière `dyn`.

Aucun type Bollard n'apparaît dans `hormos-core`, ni dans sa signature publique :
l'adaptateur traduit les réponses et les erreurs vers les types du domaine.

## Conséquences

- **+** Testabilité (Mock), portabilité moteur, couplage faible interfaces/moteur.
- **+** Surface d'API interne explicite et contrôlée.
- **−** Une couche d'indirection à maintenir ; le trait devra être conçu avec soin
  pour ne pas fuir les détails de Docker.

## Alternatives écartées

- **Appeler Bollard directement partout** : couplage fort, tests exigeant Docker,
  migration moteur impossible.
- **Shell-out vers la CLI `docker` pour tout** : fragile, difficile à typer, et
  incompatible avec l'objectif de contrôle fin (préféré uniquement pour Compose,
  voir [ADR 0002](0002-compose-driver.md)).

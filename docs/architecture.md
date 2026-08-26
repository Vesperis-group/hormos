# Architecture

> « One engine. Every interface. »

Hormos est un *control plane* conteneurs **local-first** et **security-first**.
Ce document décrit l'architecture **cible** ; à ce stade, la CLI, le TUI, le cœur
et l'adaptateur Docker existent — les autres interfaces restent à venir.

## Principe fondateur

Un **seul cœur** Rust contient toute la logique métier. Les interfaces (CLI,
TUI, API HTTP, Web) sont de simples adaptateurs : elles n'embarquent **aucune**
logique Docker et se contentent d'appeler le cœur.

```
 Interfaces (adaptateurs, sans logique Docker)
   CLI (hormos)   TUI        API (Axum)     Web (React, servi par l'API)
        \          \            /              /
         \          \          /              /
          +----------------------------------+
          |            hormos-core           |
          |  - orchestration / cas d'usage   |
          |  - trait ContainerRuntime        |
          |  - driver Compose (séparé)       |
          +----------------------------------+
                 |                    |
        ContainerRuntime         Compose driver
                 |                    |
          Docker (Bollard)      `docker compose`
          [futur : Podman,      (jamais réimplémenté)
           Mock pour les tests]
```

## Crates

État actuel :

- `crates/hormos-core` : domaine, validation des références, modèle d'erreurs,
  trait `ContainerRuntime` et `ContainerService` (cas d'usage). **Ne dépend
  d'aucun client Docker.**
- `crates/hormos-docker` : implémentation du trait via **Bollard**, sur socket
  Unix local uniquement. Seul crate à connaître Docker.
- `crates/hormos-cli` : binaire `hormos`. Analyse d'arguments, rendu, codes de
  sortie. Ne nomme `hormos-docker` qu'à son point de composition (`main`).
- `crates/hormos-tui` : interface terminal (ratatui). **Ne dépend ni de Bollard
  ni de `hormos-docker`** : elle reçoit un `ContainerService` déjà construit.
  Voir [tui.md](tui.md).

À venir (non créés tant qu'ils ne sont pas réellement utiles) :

- driver Compose ;
- `hormos-api`, `hormos-web` : adaptateurs.

Aucune crate vide n'est créée d'avance (anti-surengineering).

## Flux d'un appel

```
hormos ps --all
  → hormos-cli      : analyse des arguments, choix du rendu
  → ContainerService: validation de l'entrée (avant toute connexion)
  → dyn ContainerRuntime
  → DockerRuntime   : délai maximal, appel Bollard, traduction erreurs
  → Bollard         : socket Unix local
```

Le service **valide avant de déléguer** : une référence invalide n'atteint
jamais le moteur, et n'ouvre même pas le socket.

Le TUI emprunte exactement le même chemin : ses touches produisent des messages,
que son état pur traduit en commandes exécutées **hors du rendu** par le même
`ContainerService`. Un second adaptateur, aucun second cœur.

## Abstraction runtime

Le moteur de conteneurs est masqué derrière le trait `ContainerRuntime`
(voir [ADR 0001](adr/0001-runtime-abstraction.md)). La première implémentation
cible **Docker via Bollard**. Le trait étant *object-safe*, le cœur manipule un
`Arc<dyn ContainerRuntime>` : un faux déterministe (tests) ou une future
implémentation Podman s'y branchent sans toucher aux interfaces.

### Flux temps réel

Les journaux et les événements traversent la même abstraction, `RuntimeStream<T>`
(voir [streams.md](streams.md)). Elle est **opaque** : les interfaces ne voient
ni `futures`, ni Bollard, ni HTTP. Le trait reste *object-safe* parce que le type
renvoyé est concret et contient un flux boxé — un `impl Stream` ne passerait pas
derrière `dyn`.

Un flux **n'accumule rien** ; toute la rétention appartient à ce qui affiche. Le
détruire l'annule, sans jeton ni message d'arrêt. Sa requête n'est émise qu'à la
première lecture.

### Politique de délais

Aucune opération **ponctuelle** ne peut bloquer indéfiniment : chaque appel est
encadré par un délai maximal fixe (`crates/hormos-docker/src/timeouts.rs`). Le
délai du client HTTP est volontairement supérieur à tous les autres, afin que ce
soit toujours Hormos qui tranche, avec un message clair, et jamais un abandon
opaque du client.

Les **flux** en sont exclus, et c'est voulu : `hormos logs -f` doit pouvoir vivre
des heures. Leur bornage est ailleurs — mémoire bornée côté affichage, et
annulation à la main de l'utilisateur.

## Compose

Compose est un **driver séparé** qui **invoque `docker compose`** en
sous-processus (voir [ADR 0002](adr/0002-compose-driver.md)). Hormos ne
réimplémente pas le format Compose et n'utilise jamais `sh -c` ni d'interpolation
shell : les arguments sont passés en tableau explicite.

## Modèle de service Web

Le Web sera **servi par l'API Axum** (fichiers statiques + endpoints), sans
conteneur frontend obligatoire (voir [ADR 0003](adr/0003-web-serving-model.md)).
Les échanges interactifs (logs, exec, events) passeront par REST + SSE/WebSocket,
uniquement là où l'interactivité l'exige.

## Réseau & persistance

- Bind par défaut : `127.0.0.1` (jamais `0.0.0.0` par défaut).
- API **locale et explicite** : jamais un proxy transparent du socket Docker.
- **Docker reste la source de vérité.** Pas de base de données introduite
  prématurément.

## Voir aussi

- [tui.md](tui.md)
- [streams.md](streams.md)
- [security-model.md](security-model.md)
- [threat-model.md](threat-model.md)
- [ADR](adr/)

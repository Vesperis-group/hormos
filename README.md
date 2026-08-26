# Hormos

> One engine. Every interface.

**Hormos** est un *control plane* **local-first** et **security-first** pour
conteneurs. Un même cœur Rust est destiné à être consommé, à terme, par
plusieurs interfaces (CLI, TUI, API HTTP, Web) — sans dupliquer la logique
métier.

> ⚠️ **État : early development.** Hormos sait aujourd'hui **observer et piloter
> le cycle de vie** des conteneurs d'un moteur Docker **local** (`info`, `ps`,
> `inspect`, `start`, `stop`, `restart`), et **suivre** journaux et événements
> (`logs`, `events`), en ligne de commande comme dans une interface terminal. Il
> ne sait ni créer, ni supprimer un conteneur, ni gérer images, volumes, réseaux
> ou Compose.

## Vision

Hormos vise à offrir une administration de conteneurs :

- **local-first** : pas de dépendance cloud, Docker reste la source de vérité ;
- **security-first** : surface d'attaque minimale, pas de proxy Docker brut, bind
  par défaut sur `127.0.0.1`, socket Docker traité comme un privilège quasi-root ;
- **multi-interface** : « one engine, every interface » — CLI, TUI, API et Web
  partagent le même cœur.

Voir [`docs/architecture.md`](docs/architecture.md) et les
[ADR](docs/adr/) pour les décisions structurantes.

## Utilisation

Pré-requis : un moteur **Docker local** accessible par socket Unix, et un
utilisateur autorisé à le lire (typiquement, appartenance au groupe `docker`).

```bash
hormos                      # interface terminal (identique à `hormos tui`)

hormos info                 # version du moteur, API négociée, compteurs
hormos ps                   # conteneurs en cours d'exécution
hormos ps --all             # y compris les conteneurs arrêtés
hormos inspect <ref>        # détail minimal d'un conteneur
hormos start <ref>          # idempotent
hormos stop <ref>           # idempotent
hormos restart <ref>

hormos logs <ref>           # journal complet, puis rend la main
hormos logs <ref> -f        # suit jusqu'à Ctrl+C
hormos logs <ref> --tail 20 --timestamps
hormos events               # événements du moteur, jusqu'à Ctrl+C

hormos info --json          # sortie scriptable (aussi sur `ps` et `inspect`)
hormos events --json        # NDJSON : un objet complet par ligne
```

`<ref>` est un **nom** ou un **identifiant** de conteneur. Les références sont
validées avant tout appel au moteur.

Interrompre un suivi par `Ctrl+C` est un **succès** (code `0`). Vers un terminal,
le journal est assaini ; vers un fichier ou un tube, les octets sont recopiés à
l'identique, pour ne casser aucune chaîne de traitement. Voir
[`docs/streams.md`](docs/streams.md).

### Interface terminal

`hormos` sans argument ouvre une vue plein écran des conteneurs : navigation au
clavier, filtre, détail, `start` / `stop` / `restart`, journal (`l`) et
événements (`e`). Elle n'interroge le moteur que sur action explicite — aucun
sondage périodique, aucune reconnexion automatique. Hors d'un terminal
interactif (redirection, tube, CI), elle refuse de démarrer avec le code `2`,
avant même d'ouvrir le socket.

Touches et garanties : [`docs/tui.md`](docs/tui.md).

### Point de terminaison

Hormos ne se connecte qu'à un socket **Unix local**, résolu dans cet ordre :

1. `DOCKER_HOST`, qui doit être de la forme `unix:///chemin/vers/docker.sock` ;
2. `$XDG_RUNTIME_DIR/docker.sock` (Docker *rootless*) ;
3. `/var/run/docker.sock`.

Un `DOCKER_HOST` en `tcp://`, `http://`, `https://`, `ssh://` ou `npipe://` est
**refusé explicitement** (code de sortie `8`) : les transports distants ne sont
même pas compilés dans le binaire.

### Codes de sortie

| Code | Signification                       |
| ---- | ----------------------------------- |
| `0`  | Succès                              |
| `1`  | Erreur moteur non classée           |
| `2`  | Usage ou référence invalide         |
| `3`  | Moteur injoignable                  |
| `4`  | Accès refusé (permissions socket)   |
| `5`  | Conteneur introuvable               |
| `6`  | Conflit d'état                      |
| `7`  | Délai dépassé                       |
| `8`  | Transport ou moteur non supporté    |

## Architecture

```
            +-----------------------------+
 CLI        |         hormos-core          |
 TUI        |  ContainerRuntime (trait)   |-> hormos-docker (Bollard, socket local)
 (API, Web  |  Compose = driver séparé     |-> `docker compose` (à venir)
  à venir)->+-----------------------------+
```

- Les interfaces ne contiennent **aucune** logique Docker : elles passent par
  `hormos-core`, qui ne connaît que le trait `ContainerRuntime`.
- `hormos-tui` ne dépend ni de Bollard ni de `hormos-docker` : il reçoit un
  `ContainerService` déjà construit, exactement comme la CLI.
- `hormos-docker` est le **seul** crate à dépendre de Bollard, et il n'est
  compilé qu'avec le transport socket local.
- Compose sera un **driver séparé** qui appelle `docker compose` — jamais
  réimplémenté, jamais via `sh -c`.
- Le Web sera servi par l'API (Axum) — pas de conteneur frontend obligatoire.

Voir [`docs/adr`](docs/adr/) pour les décisions structurantes.

## ⚠️ Avertissement de sécurité — socket Docker

Accéder au socket Docker (`/var/run/docker.sock`) équivaut à un privilège
**quasi-root** sur l'hôte. Hormos ne l'exposera jamais tel quel sur le réseau et
n'agira jamais comme un proxy Docker brut. Voir
[`docs/security-model.md`](docs/security-model.md) et
[`docs/threat-model.md`](docs/threat-model.md).

## Build / développement

Pré-requis : la toolchain Rust est épinglée par [`rust-toolchain.toml`](rust-toolchain.toml)
(`1.98.0`, edition 2024). `rustup` la sélectionne automatiquement.

```bash
# Compilation + tests
cargo build --workspace --locked
cargo test --workspace --locked

# Tests contre un vrai moteur Docker (désactivés par défaut)
HORMOS_DOCKER_INTEGRATION=1 cargo test -p hormos-docker --test engine -- --test-threads=1

# Quality gate locale
make check     # fmt + clippy + tests
make audit     # cargo audit / deny / machete / gitleaks (si présents)

# Binaire
cargo run -p hormos-cli -- info
```

Voir [`docs/development.md`](docs/development.md) pour le détail.

## Versioning

- Version courante : **`0.0.0`**. La `0.1.0` sera produite par le pipeline de
  release à partir des commits `feat:` (voir [`docs/release.md`](docs/release.md)).
- Cargo/workspace est la **source de vérité** de la version.
- Commits : [Conventional Commits](https://www.conventionalcommits.org/), signés.

## Licence

[MIT](LICENSE) © Vesperis-group.

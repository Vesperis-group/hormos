# Développement

## Prérequis

- **Rust** : épinglé par [`rust-toolchain.toml`](../rust-toolchain.toml)
  (`1.98.0`, edition 2024). `rustup` sélectionne la bonne toolchain
  automatiquement, en local comme en CI.
- Cible musl (release/CI) : `rustup target add x86_64-unknown-linux-musl`
  (déjà déclarée dans `rust-toolchain.toml`).
- Outils d'audit optionnels (pour `make audit` / `make security-full`) :
  `cargo-audit 0.22.2`, `cargo-deny 0.20.2`, `cargo-machete 0.9.2`, `gitleaks`,
  `shellcheck`, `actionlint`.
- Outillage de release (Node) : voir [release.md](release.md).

## Boucle de développement

```bash
# Compilation / exécution
cargo build --workspace --locked
cargo run -p hormos-cli -- info
cargo run -p hormos-cli            # interface terminal

# Tests (sans Docker)
cargo test --workspace --all-features --locked

# Quality gate (identique à la CI)
make check      # fmt + clippy strict + tests
```

Détail des commandes CI :

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --all-features --locked --release
```

## Travailler sur l'interface terminal

`crates/hormos-tui` sépare volontairement l'état du rendu :

- `app.rs` est **pur** — pas de terminal, pas de Docker, pas d'horloge. Tout
  comportement s'y teste en envoyant un `Message` et en observant l'état et
  l'éventuelle `Command` renvoyée. **Toute nouvelle interaction se teste ici**,
  pas à l'écran ;
- `ui.rs` se teste avec le `TestBackend` de ratatui : on rend dans un tampon en
  mémoire et on affirme sur le texte obtenu, à plusieurs tailles ;
- `stream.rs` porte les tampons bornés et le défilement ; il ne connaît ni
  terminal ni moteur, et se teste en poussant des éléments ;
- `event.rs` et `terminal.rs` sont les seuls à toucher au terminal réel ; ils
  restent minces à dessein.

Le crate ne doit **jamais** acquérir de dépendance Docker. Le garde-fou :

```bash
cargo tree -p hormos-tui -e normal | grep -E "bollard|hormos-docker"   # doit être vide
```

Essai manuel :

```bash
cargo run -p hormos-cli            # ou `cargo run -p hormos-cli -- tui`
```

Voir [tui.md](tui.md) pour les touches et les garanties, et
[streams.md](streams.md) pour les flux.

Un test de flux ne doit **jamais** attendre un temps fixe pour conclure : on
pousse des messages dans l'état et on affirme sur ce qu'il en fait. Les
générations rendent la stalité testable — un message d'une génération périmée
doit être ignoré, et c'est le genre d'assertion qui protège le mieux la boucle.

## Tests contre un vrai moteur Docker

Les tests de `crates/hormos-docker/tests/engine.rs` parlent à un **vrai** démon.
Ils ne s'exécutent que si `HORMOS_DOCKER_INTEGRATION=1` est défini : par défaut,
`cargo test --workspace` reste vert sur une machine sans Docker et ne touche
jamais au moteur d'un poste de développement.

```bash
HORMOS_DOCKER_INTEGRATION=1 cargo test -p hormos-docker --test engine -- --test-threads=1
```

Règles à respecter pour tout nouveau test d'intégration :

- l'image est **épinglée par digest** — jamais `latest`, jamais une balise seule ;
- chaque conteneur porte trois étiquettes : `io.hormos.test=true` (reconnaissance),
  `io.hormos.test.run=<exécution>` (identité de la suite) et
  `io.hormos.test.fixture=<conteneur>` (identité individuelle) ;
- toute sélection destinée à une suppression croise **au moins** `io.hormos.test=true`
  et l'identité d'exécution : filtrer sur la seule étiquette générique atteindrait
  les conteneurs d'une autre suite Hormos partageant le démon ;
- le nettoyage est garanti même en cas d'échec (garde `Drop`) et ne cible que la
  fixture concernée ;
- **jamais** de `docker prune`, jamais de suppression d'image, jamais de
  modification d'un conteneur préexistant ;
- la fixture est créée via le client `docker` en sous-processus, arguments passés
  en tableau, sans shell : créer et supprimer des conteneurs est hors du
  périmètre actuel d'Hormos ;
- un test de flux borne son attente par une **échéance** et ne conclut jamais sur
  un `sleep`. Pour observer des événements, il faut **commencer à lire avant** de
  provoquer ce qu'on veut voir : un flux n'émet sa requête qu'à la première
  lecture, et un événement manqué ne se rejoue pas. Pour conclure au silence d'un
  journal suivi, il faut d'abord le **drainer** : le moteur découpe l'historique
  en un nombre de fragments qu'il choisit seul.

L'identité d'exécution vient de `HORMOS_DOCKER_TEST_RUN_ID` lorsqu'elle est
fournie (c'est le cas en CI) ; sinon la suite en dérive une, unique et
**stable pour tout le processus de test**, afin qu'un nettoyage global de
l'exécution reste possible. La valeur est restreinte à `[A-Za-z0-9._-]`.

Après exécution, aucun conteneur ne doit subsister **pour cette exécution** :

```bash
docker ps -a --filter label=io.hormos.test=true \
             --filter "label=io.hormos.test.run=$HORMOS_DOCKER_TEST_RUN_ID"
```

Ne demandez pas que `docker ps -a --filter label=io.hormos.test=true` soit
globalement vide : une autre suite légitime peut tourner sur le même démon, et
ses conteneurs ne vous appartiennent pas.

## Audit local

```bash
make audit          # tolérant : ignore proprement les outils absents
make security-full  # strict : échoue si un outil requis manque
```

## Conventions de code

- `unsafe` interdit (`unsafe_code = "forbid"`).
- Pas d'`unwrap()`/`expect()` injustifié en production (lints `warn`, promus en
  erreur par `-D warnings`). Dans les tests, `expect` est autorisé via un
  `#![allow(clippy::expect_used)]` local.
- Formatage `rustfmt` par défaut. Commentaires uniquement là où c'est utile.

## Fuzzing (reporté)

Aucune cible `cargo-fuzz` n'est ajoutée au bootstrap : il n'existe pas encore de
surface de parsing/entrée non triviale à fuzzer. Le fuzzing sera introduit dès
qu'une telle surface apparaîtra (par ex. parsing d'un fichier Compose ou d'une
entrée réseau). Documenter ce report évite le fuzz « cosmétique » sans valeur.

## Périmètre — ne pas ajouter maintenant

Création/suppression de conteneurs, `exec`, logs, stats, images, volumes,
réseaux, événements, Ratatui, Axum, React/Vite, driver Compose, SQLite, support
d'un moteur distant, ou toute crate vide destinée à ces futures fonctionnalités.
Voir [ADR](adr/) et [`../CONTRIBUTING.md`](../CONTRIBUTING.md).

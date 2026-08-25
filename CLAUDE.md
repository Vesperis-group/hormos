# CLAUDE.md

Invariants durables d'Hormos. Court par conception : pas de détail volatil.

## Architecture

- « One engine. Every interface. » Un seul cœur Rust ; CLI/TUI/API/Web ne
  contiennent **aucune** logique Docker.
- Runtime derrière un trait `ContainerRuntime` (Docker/Bollard d'abord).
- Compose = driver séparé appelant `docker compose` ; jamais réimplémenté.
- Web servi par l'API (Axum) à terme ; pas de conteneur frontend obligatoire.
- Docker reste la source de vérité : pas de base de données prématurée.

## Sécurité

- Socket Docker = privilège quasi-root : jamais exposé brut, jamais proxifié.
- Bind par défaut `127.0.0.1`. Pas de `sh -c` ni d'interpolation shell.
- `unsafe` interdit (`unsafe_code = "forbid"`). Pas d'`unwrap()`/`expect()`
  injustifié en production.

## Git / PR

- Toujours une PR vers `main`. Aucun push direct (seule exception historique : le
  seed initial du dépôt vide).
- Conventional Commits, **atomiques et signés**.
- Pas de `feat:` tant qu'une release *minor* n'est pas voulue.

## Supply chain

- Actions GitHub **épinglées par SHA** complet + commentaire `# vX.Y.Z`.
- Permissions Actions minimales : `contents: read` par défaut, élévation par job.
- Dépendances épinglées ; SBOM + signature cosign + provenance à la release.
- Release : commit **Verified** créé par la GitHub App via l'API (pas de PAT, pas
  de clé dédiée) ; build depuis le tag exact ; provenance sur le SHA construit ;
  identité Sigstore restreinte à `release.yml@refs/heads/main`.

## Anti-surengineering

- Ne rien ajouter « pour plus tard » : pas de crate vide, pas de Bollard/Compose/
  Ratatui/Axum/React/SQLite avant qu'une vraie surface l'exige.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --all-features --locked --release
cargo audit && cargo deny check && cargo machete
```

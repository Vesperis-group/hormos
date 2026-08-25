# Contribuer à Hormos

Merci de votre intérêt. Ce projet est en *early development* : les contributions
sont bienvenues mais doivent respecter des règles strictes de sécurité et de
qualité.

## Prérequis

- Toolchain Rust épinglée par [`rust-toolchain.toml`](rust-toolchain.toml)
  (`rustup` la sélectionne seule).
- Optionnel (audit local) : `cargo-audit`, `cargo-deny`, `cargo-machete`,
  `gitleaks`, `shellcheck`, `actionlint`.

## Workflow Git

- **Toujours** passer par une Pull Request vers `main`. Aucun push direct.
- Une branche par sujet : `feat/…`, `fix/…`, `chore/…`, `docs/…`.
- Gardez la branche à jour avec `main` (rebase/merge) avant la revue finale.

## Commits

- [Conventional Commits](https://www.conventionalcommits.org/) **obligatoires** :
  `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`, `ci:`, `build:`…
- Commits **atomiques** et **signés** (`git commit -S`). Les commits non signés
  seront rejetés par les rulesets (voir [`docs/repository-rules.md`](docs/repository-rules.md)).
- `feat:` déclenche une *minor* et `fix:` une *patch* à la release : ne les
  utilisez que lorsque le comportement change réellement.
- Ne committez **jamais** de secret ni d'artefact généré.

## Quality gate (avant de pousser)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --all-features --locked --release
```

Ou simplement :

```bash
make check      # fmt + clippy + tests
make audit      # audit supply chain (si les outils sont installés)
```

La CI rejoue tout : aucune PR n'est mergée si un check échoue.

## Style de code

- `unsafe` est **interdit** (`unsafe_code = "forbid"` au niveau workspace).
- Pas d'`unwrap()`/`expect()` injustifié en production (lint `warn`).
- Commentez uniquement ce qui a besoin d'être clarifié.

## Périmètre

Ce dépôt suit une feuille de route explicite. N'introduisez pas prématurément :
runtime Docker/Bollard, Compose driver, TUI (Ratatui), API (Axum), Web
(React/Vite), base de données. Voir les [ADR](docs/adr/).

## Signalement de vulnérabilité

Voir [`SECURITY.md`](SECURITY.md). Ne créez pas d'issue publique pour une faille.

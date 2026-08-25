## Description

<!-- Que fait cette PR et pourquoi ? -->

## Type de changement

- [ ] `fix:` correctif (patch)
- [ ] `feat:` nouvelle fonctionnalité (minor) — ⚠️ déclenche une release minor
- [ ] `chore:` / `docs:` / `refactor:` / `ci:` / `test:` (pas de release)

## Checklist

- [ ] Commits **atomiques**, **signés** et en **Conventional Commits**.
- [ ] `cargo fmt --all -- --check` OK
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` OK
- [ ] `cargo test --workspace --all-features --locked` OK
- [ ] Aucun secret ni artefact généré committé.
- [ ] Documentation mise à jour si nécessaire.
- [ ] Périmètre respecté (pas de Bollard/Compose/TUI/API/Web prématurés — voir ADR).

## Notes de sécurité

<!-- Impact éventuel sur la surface d'attaque, les permissions, la supply chain. -->

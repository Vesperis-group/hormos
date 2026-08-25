# Changelog

Toutes les modifications notables de ce projet sont documentées ici.

Le format s'appuie sur [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet suit [Semantic Versioning](https://semver.org/lang/fr/). Les entrées
sont générées à partir des [Conventional Commits](https://www.conventionalcommits.org/)
lors de la release (`release-it`).

## [Non publié]

### Ajouté

- Bootstrap du dépôt : workspace Rust (edition 2024, MSRV 1.98.0), CLI `hormos`
  minimale (`--version` / `--help`).
- Fondation DevSecOps : CI (fmt, clippy, test, build), audit supply chain
  (cargo-audit / deny / machete, gitleaks), lint (actionlint, ShellCheck),
  CodeQL, OpenSSF Scorecard, Dependabot.
- Outillage de release Linux x86_64 (glibc + musl) : archives, SHA-256, SBOM
  CycloneDX, signature cosign keyless et provenance SLSA — désactivé par défaut
  tant que `HORMOS_RELEASE_ENABLED` n'est pas positionné.
- Documentation d'architecture, modèle de sécurité, threat model et ADR.

[Non publié]: https://github.com/Vesperis-group/hormos/commits/main

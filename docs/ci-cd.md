# CI / CD

Toute la CI vit dans [`.github/workflows`](../.github/workflows). Principes
transverses :

- **Permissions minimales** : `permissions: contents: read` par défaut au niveau
  workflow ; élévation seulement par job qui en a besoin.
- **Actions épinglées par SHA** de commit complet, avec commentaire `# vX.Y.Z`.
  Aucun tag flottant. Dependabot met à jour le SHA **et** le commentaire.
- **`--locked`** partout : les builds échouent si `Cargo.lock` est incohérent.
- **Concurrency** par workflow/ref pour annuler les runs obsolètes.

## `ci.yml` — qualité

Job `rust`, sur PR et push `main` :

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --all-features --locked`
- `cargo build --workspace --all-features --locked --release`
- `git diff --check` (espaces/fins de ligne parasites)

Job `docker-integration`, sur le même déclencheur : exécute
`crates/hormos-docker/tests/engine.rs` contre le démon Docker fourni par le
runner GitHub, avec `HORMOS_DOCKER_INTEGRATION=1`. L'image de test est épinglée
par digest.

Le job publie `HORMOS_DOCKER_TEST_RUN_ID`
(`<run_id>-<run_attempt>-<job>`, aucune donnée sensible) : les conteneurs créés
en portent l'étiquette, et le nettoyage `if: always()` ne sélectionne que
`label=io.hormos.test=true` **et** `label=io.hormos.test.run=<cette exécution>`.
Une suite Hormos concurrente sur le même démon ne peut donc pas être supprimée —
**jamais** de `docker prune`. L'étape finale échoue s'il subsiste un conteneur
de **cette** exécution ; elle n'exige rien des autres.

## `audit.yml` — supply chain

Sur PR et push `main` (+ `workflow_dispatch`) :

- `cargo audit` — versions épinglées : `cargo-audit 0.22.2`, `cargo-deny 0.20.2`,
  `cargo-machete 0.9.2`.
- `cargo deny check` (licences, sources, avis).
- `cargo machete` (dépendances inutilisées).
- `gitleaks` — binaire épinglé (`8.27.2`) et archive vérifiée par SHA-256.

## `lint.yml` — infrastructure

- `actionlint` (`1.7.7`) — checksum vérifié — sur tous les workflows.
- ShellCheck (`0.11.0`) — checksum vérifié — sur les scripts de release.

## `codeql.yml` — SAST

CodeQL pour Rust, permissions minimales (`security-events: write`), actions
épinglées, sur PR/push/planifié.

## `scorecard.yml` — OpenSSF Scorecard

Analyse OpenSSF Scorecard avec publication des résultats et upload SARIF vers
Code Scanning (dépôt public requis pour `publish_results: true`).

## `release.yml` — release

Voir [release.md](release.md). Déclenché sur push `main` mais **garde** : la
publication ne s'exécute que si la variable `HORMOS_RELEASE_ENABLED` vaut `true`.
Sans cette garde, l'absence de secrets ne fait **pas** échouer la CI.

## Dependabot

[`.github/dependabot.yml`](../.github/dependabot.yml) — hebdomadaire — couvre
`cargo`, `github-actions` et `npm` (outillage de release). Groupé minor/patch,
**aucun auto-merge**.

## Épinglage des actions

Les SHA proviennent des releases vérifiées de chaque action. Exemple :

```yaml
uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
```

Pour mettre à jour une action manuellement : récupérer le SHA du tag de release
correspondant (`git ls-remote`/page de release), remplacer SHA **et** commentaire.

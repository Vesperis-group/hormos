# Règles du dépôt (rulesets & protections)

Ce document **propose** les protections à appliquer sur `hormos`. Conformément au
périmètre de la PR #1, **aucun réglage GitHub n'est modifié automatiquement** :
ces règles sont à configurer manuellement une fois que les checks CI sont apparus
au moins une fois (pour pouvoir les sélectionner comme *required checks*).

## Ruleset « Protect main »

Cible : branche `main`.

- **Pull request obligatoire** avant merge (aucun push direct).
  - Au moins 1 approbation ; *dismiss stale approvals* on push.
  - **Require conversation resolution** avant merge.
- **Required status checks** (à cocher une fois visibles) :
  - CI : `Rust (fmt, clippy, test, build)`
  - Audit : `cargo audit / deny / machete`, `gitleaks`
  - Lint : `actionlint`, `ShellCheck`
  - CodeQL : `CodeQL (Rust)`
  - *Require branches to be up to date before merging*.
- **Require signed commits**.
- **Block force pushes** ; **restrict deletions**.
- **Bypass list** : **uniquement** la GitHub App `vesperis-hormos-release`
  (aucun humain, ni le `GITHUB_TOKEN` par défaut). Voir [release-app.md](release-app.md).

## Ruleset « Protect release tags »

Cible : tags `v*`.

- **Restrict creations/updates/deletions** : tags immuables une fois créés.
- **Bypass list** : **uniquement** la GitHub App `vesperis-hormos-release`
  (elle doit pouvoir créer le tag de release).

## Exception historique (seed)

Le tout premier commit du dépôt (initialisation d'un dépôt vide) est la seule
exception autorisée au workflow PR. Une fois `main` protégée, **plus aucun** push
direct n'est permis.

## Ordre d'application recommandé

1. Ouvrir la PR #1 et laisser tourner la CI une première fois.
2. Créer les deux rulesets ci-dessus, en sélectionnant les checks désormais
   visibles comme *required*.
3. Ajouter la GitHub App de release aux *bypass lists* (section correspondante de
   [release-app.md](release-app.md)).
4. N'activer la publication automatique (`HORMOS_RELEASE_ENABLED=true`) qu'après
   vérification de bout en bout de l'App.

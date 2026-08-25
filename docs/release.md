# Release

Le modèle de release d'Hormos est adapté de celui de Mnemo, avec **pnpm** comme
gestionnaire de paquets Node (jamais npm comme gestionnaire principal). Cargo /
le workspace reste la **source de vérité** de la version.

> **État : gardé/désactivé.** Tant que la variable `HORMOS_RELEASE_ENABLED`
> n'est pas positionnée à `true`, un push sur `main` **ne déclenche pas** la
> publication et **n'échoue pas** faute de secrets. La release automatique
> ne sera activée qu'après vérification complète de la GitHub App
> (voir [release-app.md](release-app.md)).

## Versioning

- Version courante : `0.0.0`. La `0.1.0` est réservée à la « Docker foundation ».
- [Conventional Commits](https://www.conventionalcommits.org/) : `feat:` →
  *minor*, `fix:` → *patch*. **La PR #1 ne contient aucun `feat:`** pour ne pas
  déclencher prématurément la `0.1.0`.
- `release-it` sert **uniquement** à calculer la version et à générer le
  changelog + le bump de `Cargo.toml` (aucune opération git/github ; voir
  lifecycle plus bas). Le tag et la GitHub Release sont créés par le workflow.

## Outillage Node (pnpm)

- [`.node-version`](../.node-version) = `24.19.0`
- `package.json` → `"packageManager": "pnpm@11.23.0"`, dépendances de release
  **exactes/verrouillées** (`pnpm-lock.yaml`, aucune `package-lock.json`).
- Les devDependencies de release sont **exécutables et sensibles** : elles sont
  auditées (`pnpm audit`, y compris dev). On ne reprend **pas** un
  `--omit=dev`.

## Artefacts Linux x86_64

La release produit et publie, pour **glibc** et **musl** :

- archives `.tar.gz` (binaire `hormos` + README + LICENSE) ;
- empreintes `.sha256` par archive ;
- SBOM CycloneDX (`*-sbom.cdx.json` + `.sha256`) ;
- fichier de **checksums agrégés** (`*-checksums.txt`) ;
- **signatures cosign keyless** (OIDC) de chaque artefact ;
- **provenance SLSA** (`*-provenance.intoto.jsonl`).

Pas d'`arm64` pour l'instant.

## Classification des assets (déterminisme vs preuve)

La politique de recovery **diffère selon la nature de l'asset** :

| Classe | Assets | Recovery |
|--------|--------|----------|
| **Payload reproductible** | archives `.tar.gz`, `.sha256`, SBOM, `checksums.txt` | comparaison **SHA-256 stricte** (même release SHA ⇒ octets identiques ; mismatch = anomalie supply-chain → fail-close) |
| **Evidence cryptographique** | `*.sigstore.json`, `*.provenance.sigstore.json` | **vérification cosign** (identité `release.yml@main` + issuer + claims), **jamais** de comparaison octet-à-octet à une preuve régénérée |
| **Agrégat de provenance** | `*.provenance.intoto.jsonl` | **reconstruit** depuis les bundles provenance **canoniques** de la Release, puis comparé octet-à-octet (inputs figés) |

Reproductibilité mesurée : le **binaire** (glibc) est reproductible sur un même
hôte/toolchain ; les **archives** le sont grâce à un `tar` normalisé
(`SOURCE_DATE_EPOCH` dérivé du commit de release, `--sort=name`, `--owner=0
--group=0 --numeric-owner`, `gzip -n`) ; le **SBOM** est canonicalisé (retrait des
seuls champs volatils `metadata.timestamp` et `serialNumber`, sans toucher
composants/hashes/licences/PURL/relations). Le `serialNumber` est réattribué sous
forme d'un véritable **UUID v5 RFC 4122** (`uuid.uuid5`, namespace projet dérivé
de l'URL du dépôt + contenu canonique du document), puis **revalidé
explicitement** (reparsable, `version == 5`, `variant == RFC_4122`) — fail-close
sinon. CycloneDX n'exige que la conformité RFC 4122 ; le **déterminisme** de ce
serial est un **choix Hormos** : un SBOM reconstruit pour un même release SHA doit
être byte-reproductible pour permettre le recovery par SHA-256. Les preuves
Sigstore (nouveau
certificat/horodatage de transparence à chaque `sign-blob`/`attest-blob`) et la
provenance (contenant `invocationId`/`run_id`/`run_attempt`) sont **légitimement
non déterministes** : elles sont vérifiées cryptographiquement, pas par SHA.

## Scripts

Scripts (dans [`scripts/`](../scripts)), tous *fail-close* (`set -euo pipefail`) :

| Script | Rôle |
|--------|------|
| `create-release-commit.sh` | Crée le commit de release **Verified** (parent = base testée) via l'API GitHub et le publie sur une **ref de staging** (jamais `main`/tag) |
| `validate-artifacts.sh` | Valide tous les artefacts (présence, tailles, checksums, cosign) **avant** promotion |
| `promote-release.sh` | Re-vérifie `main == base`, fast-forward `main`, crée le **tag final** `vX.Y.Z` (idempotent) |
| `publish-github-release.sh` | Crée/complète la GitHub Release **idempotente**, recovery **par classe** (payload SHA / evidence cosign / intoto canonique) |
| `package-release.sh` | Construit une archive `.tar.gz` **reproductible** + `.sha256` |
| `generate-sbom.sh` | Génère le SBOM CycloneDX **canonicalisé** (reproductible) |
| `checksums-release.sh` | Agrège et vérifie les empreintes de tous les artefacts |
| `sign-release.sh` | Signe + atteste (cosign keyless) et **vérifie**, provenance sur le SHA exact |
| `intoto-provenance.sh` | Extrait la provenance SLSA au format `.intoto.jsonl` |
| `test-sign-identity.sh` | Teste l'identité Sigstore (accepte `release.yml@main`, refuse le reste) |
| `lib/release-lib.sh` | Fonctions pures testables (SHA, allowlist, no-op, tag) |
| `lib/sbom-canonicalize.py` | Canonicalise le SBOM + identité **UUID v5 RFC 4122** déterministe (revalidée) |
| `tests/release-invariants.sh` | Tests des invariants de release (§18) |

## Lifecycle (transaction robuste)

Invariants : *source commit == source attestée == source construite*, base testée
== parent du commit de release == `main` promu, et **aucun tag `v*` avant** que
les artefacts soient validés.

```text
push main (SHA = B)  ──►  quality + audit testent EXACTEMENT B
  │
  ├─ décision : release nécessaire ? (docs:/chore:/ci: → « No release required », succès)
  │
  └─ si oui :
     prepare (release-it : bump Cargo.toml + CHANGELOG, resync Cargo.lock)
       → allowlist worktree stricte (Cargo.toml/Cargo.lock/CHANGELOG.md seuls)
       → commit Verified C (parent = B) créé via l'API → REF DE STAGING (pas main/tag)
       → checkout PROPRE de C (HEAD == C, arbre propre)
       → build glibc + musl → SBOM → checksums → cosign (release.yml@main) → provenance (SHA = C)
       → VALIDATION complète des artefacts
       → PROMOTION : re-vérifie main == B (sinon fail-close), FF main B→C, tag final vX.Y.Z→C
       → GitHub Release idempotente + upload + vérification post-release
       → nettoyage best-effort de la ref de staging
```

Points clés :

- **Anti-TOCTOU** : le parent de C est le SHA déclencheur `github.sha` (la base
  testée), jamais un `main` flottant. Avant la promotion, on exige encore
  `main == B` : si `main` a avancé, `main advanced during release; refusing to
  promote stale release` → aucun tag, aucune Release, aucun force-push.
- **No-op = succès** : un historique sans `feat:`/`fix:` termine le job en succès
  avec « No release required », sans aucune mutation ni artefact.
- **Tag tardif** : les tags `v*` étant immuables, `vX.Y.Z` n'est créé qu'**après**
  builds + SBOM + checksums + signatures + provenance + validation.
- **Idempotence / rerun** : une ressource existante n'est réutilisée que si son
  identité correspond exactement. Si la ref de staging a disparu, le commit de
  release est retrouvé dans l'ordre **staging → tag `vX.Y.Z` → `main`** (identité
  vérifiée : parent/tree/message/Verified) — **jamais** de second commit `C2`. La
  ref de staging est **conservée en cas d'échec** (recovery) et supprimée
  seulement après **succès complet**.
- **Intégrité des assets par classe** : sur recovery d'une Release existante, un
  **payload reproductible** (archives/`.sha256`/SBOM/checksums) n'est réutilisé
  que si son **SHA-256** distant == local (sinon fail-close). Une **evidence
  Sigstore** (signature/provenance) est réutilisée si elle passe la vérification
  **cosign** (identité `release.yml@main` + issuer + claims `gitCommit`/`ref`),
  **jamais** comparée octet-à-octet à une preuve régénérée (une preuve d'une
  tentative antérieure reste valide pour le même SHA/ref, même avec un
  `invocationId` différent). L'agrégat `*.intoto.jsonl` est **reconstruit** depuis
  les bundles provenance canoniques de la Release puis comparé. Jamais de
  `--clobber` ni de suppression ; tout asset distant inattendu → fail-close.
- **`release-it`** ne fait que bump + changelog (aucune opération git/github).

## Commit de release Verified

Un `git commit` local dans le runner ne serait pas signé (aucune clé) : il ne
serait que *bypassé* par le ruleset, pas Verified. `scripts/create-release-commit.sh`
crée le commit via la **Git Data API** avec le jeton de l'App ; GitHub le signe
et le renvoie `verification.verified == true` (vérifié en fail-close par le
script). Comportement confirmé expérimentalement (auteur `vesperis-hormos-release[bot]`,
`reason=valid`).

## Workflow

[`.github/workflows/release.yml`](../.github/workflows/release.yml) :

1. **quality** — checkout du SHA déclencheur (`ref: github.sha`), fmt, clippy,
   tests, build glibc + musl, `bash -n` des scripts, tests d'invariants de release.
2. **audit** — cargo audit/deny + gitleaks sur le SHA déclencheur.
3. **publish** — uniquement si `HORMOS_RELEASE_ENABLED == 'true'` : décision
   release, préparation, commit Verified sur ref de staging, checkout du commit
   exact, build, SBOM, checksums, signatures, provenance, **validation**, puis
   **promotion** (FF main + tag final) et GitHub Release idempotente. Permissions
   du job : `contents: read`, `id-token: write` (cosign) ; toutes les écritures
   (commit/tag/release) passent par le jeton de l'App, pas par le `GITHUB_TOKEN`.
   Checkout `persist-credentials: false`. `concurrency` sérialise les runs sans
   `cancel-in-progress` (pas d'annulation après étape irréversible).

## Validation locale

```bash
make release-check   # fmt + clippy + tests + build glibc/musl + syntaxe scripts + test identité
make sbom            # si cargo-cyclonedx installé
make sign-check      # cosign + test d'identité Sigstore (signature réelle = CI/OIDC)
pnpm install --frozen-lockfile && pnpm run release:dry   # calcul version + changelog, sans effet de bord
```

La signature keyless réelle et la création du commit Verified via l'App
nécessitent respectivement l'OIDC et le secret de l'App : elles ne s'exécutent
qu'en CI (job `publish`).

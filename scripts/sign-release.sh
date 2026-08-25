#!/usr/bin/env bash
#
# Signe et atteste (provenance SLSA) les artefacts de release d'Hormos avec
# cosign en mode KEYLESS (OIDC ambiant : aucun secret long terme), puis VÉRIFIE
# chaque signature et chaque attestation. Toute vérification KO interrompt le
# script.
#
# Outil : cosign (version EXACTE épinglée, installée par le workflow CI).
#
# Contexte : ce script tourne dans le job `publish` de GitHub Actions, qui
# dispose de `id-token: write`. cosign détecte alors le fournisseur OIDC
# `github-actions` et obtient un certificat Fulcio éphémère. En local (sans
# OIDC), la signature échouerait : il n'est donc PAS appelé par `make release-check`.
#
# Pré-requis : archives, SBOM et checksums déjà générés.
#
# Variables d'environnement :
#   HORMOS_VERSION                version SemVer sans 'v'. Défaut : Cargo.toml.
#   HORMOS_SIGN_IDENTITY_REGEXP   regex de l'identité de certificat attendue.
#   HORMOS_SIGN_OIDC_ISSUER       émetteur OIDC attendu.
#
# Produit, pour CHAQUE artefact <asset> :
#   <asset>.sigstore.json              bundle de signature (cosign sign-blob)
#   <asset>.provenance.sigstore.json   bundle d'attestation SLSA provenance
#
# Comportement fail-close (set -euo pipefail).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"

read_cargo_version() {
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1
}

HORMOS_VERSION="${HORMOS_VERSION:-$(read_cargo_version)}"
HORMOS_VERSION="${HORMOS_VERSION#v}"

if [ -z "${HORMOS_VERSION}" ]; then
    echo "Erreur : HORMOS_VERSION introuvable (Cargo.toml ?)." >&2
    exit 1
fi

# Identité keyless attendue : STRICTEMENT le workflow release.yml du dépôt hormos
# sur la branche main. On refuse tout autre workflow (identité trop large).
# Ancre `.` échappés ; pas de `.+` générique.
IDENTITY_REGEXP="${HORMOS_SIGN_IDENTITY_REGEXP:-^https://github\.com/Vesperis-group/hormos/\.github/workflows/release\.yml@refs/heads/main$}"
OIDC_ISSUER="${HORMOS_SIGN_OIDC_ISSUER:-https://token.actions.githubusercontent.com}"

# Mode test (spec §6) : imprime l'identité attendue et sort, sans exiger cosign
# ni les artefacts. Utilisé par scripts/test-sign-identity.sh.
if [ "${HORMOS_SIGN_PRINT_IDENTITY:-}" = "1" ]; then
    printf '%s\n' "${IDENTITY_REGEXP}"
    exit 0
fi

PREDICATE_TYPE="slsaprovenance1"

if ! command -v cosign >/dev/null 2>&1; then
    echo "Erreur : cosign est requis pour signer et attester les artefacts." >&2
    echo "Installez la version épinglée : voir le workflow CI (job publish)." >&2
    exit 1
fi

ASSETS=(
    "hormos-v${HORMOS_VERSION}-x86_64-unknown-linux-gnu-glibc2.35.tar.gz"
    "hormos-v${HORMOS_VERSION}-x86_64-unknown-linux-musl.tar.gz"
    "hormos-v${HORMOS_VERSION}-sbom.cdx.json"
    "hormos-v${HORMOS_VERSION}-checksums.txt"
)

for asset in "${ASSETS[@]}"; do
    if [ ! -f "${asset}" ]; then
        echo "Erreur : artefact attendu introuvable : ${asset}" >&2
        exit 1
    fi
done

# Prédicat de provenance SLSA v1, construit depuis les variables GitHub Actions.
PREDICATE_FILE="$(mktemp)"
trap 'rm -f "${PREDICATE_FILE}"' EXIT

GH_SERVER="${GITHUB_SERVER_URL:-https://github.com}"
GH_REPO="${GITHUB_REPOSITORY:-Vesperis-group/hormos}"
GH_WORKFLOW_REF="${GITHUB_WORKFLOW_REF:-${GH_REPO}/.github/workflows/release.yml@refs/heads/main}"

# SHA réellement construit : impératif que la provenance référence l'arbre exact
# ayant produit les artefacts (le commit de release créé/pushé AVANT le build).
# On préfère HORMOS_RELEASE_SHA (exporté par le workflow après création du commit)
# puis, à défaut, le HEAD du checkout courant. On refuse un SHA inconnu.
GH_SHA="${HORMOS_RELEASE_SHA:-$(git rev-parse HEAD 2>/dev/null || true)}"
if [ -z "${GH_SHA}" ] || [ "${GH_SHA}" = "unknown" ]; then
    echo "Erreur : SHA de release introuvable (HORMOS_RELEASE_SHA / git HEAD)." >&2
    echo "La provenance doit référencer le commit exact construit. Abandon." >&2
    exit 1
fi
# Référence de la release : le tag vX.Y.Z (par défaut) plutôt que la branche.
GH_REF="${HORMOS_RELEASE_REF:-refs/tags/v${HORMOS_VERSION}}"
GH_RUN_ID="${GITHUB_RUN_ID:-0}"
GH_RUN_ATTEMPT="${GITHUB_RUN_ATTEMPT:-0}"

cat > "${PREDICATE_FILE}" <<JSON
{
  "buildDefinition": {
    "buildType": "https://github.com/Vesperis-group/hormos/.github/workflows/release.yml",
    "externalParameters": {
      "workflow": {
        "ref": "${GH_REF}",
        "repository": "${GH_SERVER}/${GH_REPO}",
        "path": ".github/workflows/release.yml"
      }
    },
    "internalParameters": {
      "version": "${HORMOS_VERSION}"
    },
    "resolvedDependencies": [
      {
        "uri": "git+${GH_SERVER}/${GH_REPO}@${GH_REF}",
        "digest": { "gitCommit": "${GH_SHA}" }
      }
    ]
  },
  "runDetails": {
    "builder": {
      "id": "${GH_SERVER}/${GH_WORKFLOW_REF}"
    },
    "metadata": {
      "invocationId": "${GH_SERVER}/${GH_REPO}/actions/runs/${GH_RUN_ID}/attempts/${GH_RUN_ATTEMPT}"
    }
  }
}
JSON

for asset in "${ASSETS[@]}"; do
    sig_bundle="${asset}.sigstore.json"
    prov_bundle="${asset}.provenance.sigstore.json"

    rm -f "${sig_bundle}" "${prov_bundle}"

    echo ">>> Signature keyless : ${asset}"
    cosign sign-blob \
        --yes \
        --oidc-provider github-actions \
        --bundle "${sig_bundle}" \
        "${asset}"

    echo ">>> Attestation de provenance (SLSA v1) : ${asset}"
    cosign attest-blob \
        --yes \
        --oidc-provider github-actions \
        --predicate "${PREDICATE_FILE}" \
        --type "${PREDICATE_TYPE}" \
        --bundle "${prov_bundle}" \
        "${asset}"
done

# Vérification (fail-close) : une seule vérification KO interrompt → release avortée.
for asset in "${ASSETS[@]}"; do
    sig_bundle="${asset}.sigstore.json"
    prov_bundle="${asset}.provenance.sigstore.json"

    echo ">>> Vérification de signature : ${asset}"
    cosign verify-blob \
        --bundle "${sig_bundle}" \
        --certificate-identity-regexp "${IDENTITY_REGEXP}" \
        --certificate-oidc-issuer "${OIDC_ISSUER}" \
        "${asset}"

    echo ">>> Vérification de provenance : ${asset}"
    cosign verify-blob-attestation \
        --bundle "${prov_bundle}" \
        --type "${PREDICATE_TYPE}" \
        --check-claims=true \
        --certificate-identity-regexp "${IDENTITY_REGEXP}" \
        --certificate-oidc-issuer "${OIDC_ISSUER}" \
        "${asset}"
done

echo "Signatures et attestations générées et vérifiées pour ${#ASSETS[@]} artefacts."

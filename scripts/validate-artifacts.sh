#!/usr/bin/env bash
#
# Validation COMPLÈTE des artefacts de release AVANT toute promotion (spec §13).
# Aucune promotion (main/tag/Release) ne doit avoir lieu si un artefact manque,
# est vide, mal nommé, ou si une signature/cheksum ne se vérifie pas.
#
# Vérifie : présence + taille non nulle, empreintes SHA-256, checksum agrégé,
# SBOM, signatures + attestations cosign (identité restreinte release.yml@main),
# provenance intoto, et l'absence de fichier inattendu dans le jeu d'artefacts.
#
# Variables d'environnement :
#   HORMOS_VERSION   version SemVer sans 'v'. Défaut : Cargo.toml.
#
# Fail-close : set -euo pipefail.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"

read_cargo_version() {
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1
}
HORMOS_VERSION="${HORMOS_VERSION:-$(read_cargo_version)}"
HORMOS_VERSION="${HORMOS_VERSION#v}"
[ -n "${HORMOS_VERSION}" ] || { echo "Erreur : HORMOS_VERSION introuvable." >&2; exit 1; }
command -v cosign >/dev/null 2>&1 || { echo "Erreur : cosign requis." >&2; exit 1; }

V="${HORMOS_VERSION}"
GLIBC="hormos-v${V}-x86_64-unknown-linux-gnu-glibc2.35.tar.gz"
MUSL="hormos-v${V}-x86_64-unknown-linux-musl.tar.gz"
SBOM="hormos-v${V}-sbom.cdx.json"
CHECKSUMS="hormos-v${V}-checksums.txt"
INTOTO="hormos-v${V}-provenance.intoto.jsonl"

# Artefacts signés (mêmes que scripts/sign-release.sh).
SIGNED=("${GLIBC}" "${MUSL}" "${SBOM}" "${CHECKSUMS}")

# Ensemble EXACT attendu.
EXPECTED=(
    "${GLIBC}" "${GLIBC}.sha256"
    "${MUSL}" "${MUSL}.sha256"
    "${SBOM}" "${SBOM}.sha256"
    "${CHECKSUMS}"
    "${INTOTO}"
)
for a in "${SIGNED[@]}"; do
    EXPECTED+=("${a}.sigstore.json" "${a}.provenance.sigstore.json")
done

echo "== présence + taille non nulle =="
for a in "${EXPECTED[@]}"; do
    [ -s "${a}" ] || { echo "Erreur : artefact manquant ou vide : ${a}" >&2; exit 1; }
    echo "  ok ${a}"
done

echo "== aucun fichier inattendu (hormos-v${V}-*) =="
shopt -s nullglob
for f in hormos-v"${V}"-*; do
    found=0
    for a in "${EXPECTED[@]}"; do [ "${f}" = "${a}" ] && found=1 && break; done
    [ "${found}" = 1 ] || { echo "Erreur : fichier de release inattendu : ${f}" >&2; exit 1; }
done

echo "== empreintes SHA-256 =="
sha256sum -c "${GLIBC}.sha256"
sha256sum -c "${MUSL}.sha256"
sha256sum -c "${SBOM}.sha256"
sha256sum -c "${CHECKSUMS}"

echo "== identité Sigstore attendue =="
IDENTITY="$(HORMOS_SIGN_PRINT_IDENTITY=1 bash "${SCRIPT_DIR}/sign-release.sh")"
ISSUER="${HORMOS_SIGN_OIDC_ISSUER:-https://token.actions.githubusercontent.com}"
echo "  identité=${IDENTITY}"

echo "== vérification cosign (signatures + provenance) =="
for a in "${SIGNED[@]}"; do
    cosign verify-blob \
        --bundle "${a}.sigstore.json" \
        --certificate-identity-regexp "${IDENTITY}" \
        --certificate-oidc-issuer "${ISSUER}" \
        "${a}"
    cosign verify-blob-attestation \
        --bundle "${a}.provenance.sigstore.json" \
        --type slsaprovenance1 \
        --check-claims=true \
        --certificate-identity-regexp "${IDENTITY}" \
        --certificate-oidc-issuer "${ISSUER}" \
        "${a}"
    echo "  ok signatures ${a}"
done

echo "== provenance intoto non vide =="
[ -s "${INTOTO}" ] || { echo "Erreur : provenance intoto vide." >&2; exit 1; }

echo "Validation des artefacts : OK (${#EXPECTED[@]} fichiers)."

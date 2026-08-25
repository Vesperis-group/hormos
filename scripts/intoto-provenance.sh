#!/usr/bin/env bash
#
# Génère le fichier de provenance SLSA au format in-toto JSONL à partir des
# bundles Sigstore produits par scripts/sign-release.sh.
#
# Chaque ligne est l'enveloppe DSSE (champ `dsseEnvelope`) d'un bundle
# `.provenance.sigstore.json`. Ce format (.intoto.jsonl) est reconnu par les
# outils SLSA et par le check OpenSSF Scorecard « Signed-Releases ».
#
# Ce script n'effectue aucune signature : il extrait les enveloppes déjà
# produites et vérifiées par scripts/sign-release.sh.
#
# Pré-requis : bundles .provenance.sigstore.json présents, jq disponible.
#
# Variables d'environnement :
#   HORMOS_VERSION   version SemVer sans 'v'. Défaut : lue depuis Cargo.toml.
#
# Produit, à la racine du projet :
#   hormos-v${HORMOS_VERSION}-provenance.intoto.jsonl
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

# Mêmes artefacts que scripts/sign-release.sh.
ASSETS=(
    "hormos-v${HORMOS_VERSION}-x86_64-unknown-linux-gnu-glibc2.35.tar.gz"
    "hormos-v${HORMOS_VERSION}-x86_64-unknown-linux-musl.tar.gz"
    "hormos-v${HORMOS_VERSION}-sbom.cdx.json"
    "hormos-v${HORMOS_VERSION}-checksums.txt"
)

INTOTO_FILE="hormos-v${HORMOS_VERSION}-provenance.intoto.jsonl"
rm -f "${INTOTO_FILE}"

for asset in "${ASSETS[@]}"; do
    prov_bundle="${asset}.provenance.sigstore.json"
    if [ ! -f "${prov_bundle}" ]; then
        echo "Erreur : bundle de provenance introuvable : ${prov_bundle}" >&2
        echo "Exécutez scripts/sign-release.sh avant ce script." >&2
        exit 1
    fi
done

for asset in "${ASSETS[@]}"; do
    prov_bundle="${asset}.provenance.sigstore.json"
    jq -c '.dsseEnvelope' "${prov_bundle}" >> "${INTOTO_FILE}"
    echo "  Extrait : ${prov_bundle}"
done

if [ ! -s "${INTOTO_FILE}" ]; then
    echo "Erreur : ${INTOTO_FILE} est absent ou vide." >&2
    exit 1
fi

case "${INTOTO_FILE}" in
    *.intoto.jsonl) ;;
    *) echo "Erreur : le fichier de provenance doit finir par .intoto.jsonl" >&2; exit 1 ;;
esac

LINE_COUNT="$(wc -l < "${INTOTO_FILE}")"
echo "Provenance SLSA générée : ${INTOTO_FILE} (${LINE_COUNT} enveloppes DSSE)"

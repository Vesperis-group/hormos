#!/usr/bin/env bash
#
# Agrège les empreintes SHA-256 de TOUS les artefacts de release d'Hormos dans un
# unique fichier de checksums, puis vérifie l'intégralité des empreintes.
#
# Pré-requis : archives (package-release.sh) et SBOM (generate-sbom.sh) générés.
#
# Variables d'environnement (optionnelles) :
#   HORMOS_VERSION   version SemVer sans 'v'. Défaut : lue depuis Cargo.toml.
#
# Produit, à la racine du projet :
#   hormos-v${HORMOS_VERSION}-checksums.txt
#
# Comportement fail-close : artefact manquant ou empreinte non vérifiable = échec.

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

CHECKSUMS="hormos-v${HORMOS_VERSION}-checksums.txt"

# Liste EXACTE des artefacts attendus (mêmes noms que les scripts amont).
ASSETS=(
    "hormos-v${HORMOS_VERSION}-x86_64-unknown-linux-gnu-glibc2.35.tar.gz"
    "hormos-v${HORMOS_VERSION}-x86_64-unknown-linux-musl.tar.gz"
    "hormos-v${HORMOS_VERSION}-sbom.cdx.json"
)

for asset in "${ASSETS[@]}"; do
    if [ ! -f "${asset}" ]; then
        echo "Erreur : artefact attendu introuvable : ${asset}" >&2
        echo "Générez d'abord les archives et le SBOM." >&2
        exit 1
    fi
done

rm -f "${CHECKSUMS}"

sha256sum "${ASSETS[@]}" > "${CHECKSUMS}"

# Vérification immédiate : recalcule et compare toutes les empreintes.
sha256sum -c "${CHECKSUMS}"

echo "Checksums agrégés : ${CHECKSUMS}"
cat "${CHECKSUMS}"

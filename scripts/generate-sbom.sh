#!/usr/bin/env bash
#
# Génère le SBOM (Software Bill of Materials) d'Hormos au format CycloneDX JSON.
#
# Outil : cargo-cyclonedx (version EXACTE épinglée, installée par le Makefile ou
# le workflow CI ; ce script ne l'installe pas).
#
# Pré-requis : `cargo cyclonedx` dans le PATH, Cargo.lock présent.
#
# Variables d'environnement (optionnelles) :
#   HORMOS_VERSION   version SemVer sans 'v'. Défaut : lue depuis Cargo.toml.
#
# Produit, à la racine du projet :
#   hormos-v${HORMOS_VERSION}-sbom.cdx.json
#   hormos-v${HORMOS_VERSION}-sbom.cdx.json.sha256
#
# Ces fichiers sont ignorés par git : ils sont attachés à la GitHub Release.
#
# Comportement fail-close (set -euo pipefail) : toute erreur (outil absent, SBOM
# invalide, checksum KO) interrompt le script → en release, release-it avorte.

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

if ! cargo cyclonedx --version >/dev/null 2>&1; then
    echo "Erreur : cargo-cyclonedx est requis pour générer le SBOM." >&2
    echo "Installez la version épinglée : voir Makefile (make sbom) ou le workflow CI." >&2
    exit 1
fi

# --override-filename "<nom>.cdx" produit "<nom>.cdx.json" (l'outil ajoute .json).
BASENAME="hormos-v${HORMOS_VERSION}-sbom.cdx"
SBOM="${BASENAME}.json"

rm -f "${SBOM}" "${SBOM}.sha256"

# Dans un workspace, cargo-cyclonedx écrit le SBOM à côté du Cargo.toml du membre
# (ex. crates/hormos-cli/<BASENAME>.json), pas à la racine. On génère puis on
# ramène le fichier à la racine (emplacement attendu par le packaging/checksums).
cargo cyclonedx \
    --format json \
    --spec-version 1.5 \
    --all \
    --override-filename "${BASENAME}" \
    --manifest-path Cargo.toml

if [ ! -f "${SBOM}" ]; then
    # Recherche du SBOM généré dans l'arborescence (membre du workspace).
    generated="$(find . -type f -name "${SBOM}" -not -path './target/*' 2>/dev/null | head -n1)"
    if [ -n "${generated}" ] && [ "${generated}" != "./${SBOM}" ]; then
        mv "${generated}" "${SBOM}"
    fi
fi

if [ ! -f "${SBOM}" ]; then
    echo "Erreur : SBOM non généré : ${SBOM}" >&2
    exit 1
fi

# Validation + CANONICALISATION du SBOM pour le rendre REPRODUCTIBLE.
#
# cargo-cyclonedx n'a pas d'option de sortie déterministe : chaque génération
# produit un `serialNumber` (UUID aléatoire) et un `metadata.timestamp` (horodatage)
# différents. Empiriquement, deux SBOM du même projet sont IDENTIQUES une fois ces
# deux seuls champs neutralisés. On ne retire donc QUE ces champs volatils
# (jamais un composant, une version, un hash, une relation, une licence) :
#   - metadata.timestamp : supprimé ;
#   - serialNumber : remplacé par un véritable UUID v5 RFC 4122 déterministe,
#     dérivé du contenu canonique (namespace projet + document sans champs volatils).
# La logique vit dans lib/sbom-canonicalize.py (testable, revalidation explicite
# version/variant, fail-close). Elle réécrit le SBOM en JSON canonique (clés
# triées, séparateurs fixes) => octets stables pour un même contenu.
python3 "${SCRIPT_DIR}/lib/sbom-canonicalize.py" "${SBOM}"

sha256sum "${SBOM}" > "${SBOM}.sha256"
sha256sum -c "${SBOM}.sha256"

echo "SBOM créé : ${SBOM}"
cat "${SBOM}.sha256"

#!/usr/bin/env bash
#
# Construit une archive de release Linux d'Hormos, paramétrée par cible.
#
# Pré-requis : le binaire doit déjà être compilé (ce script ne compile pas).
#
# Variables d'environnement (optionnelles, valeurs par défaut pour l'usage
# local) :
#   HORMOS_VERSION       version SemVer sans 'v' (ex. 0.1.2).
#                        Défaut : lue depuis Cargo.toml (workspace).
#   HORMOS_TARGET_LABEL  étiquette de cible dans le nom de l'asset
#                        (ex. x86_64-unknown-linux-musl).
#                        Défaut : x86_64-unknown-linux-gnu.
#   HORMOS_BINARY_PATH   chemin du binaire hormos à empaqueter.
#                        Défaut : target/release/hormos.
#
# Produit, à la racine du projet :
#   hormos-v${HORMOS_VERSION}-${HORMOS_TARGET_LABEL}.tar.gz
#   hormos-v${HORMOS_VERSION}-${HORMOS_TARGET_LABEL}.tar.gz.sha256
#
# L'archive contient : hormos, README.md, LICENSE. Ces fichiers générés sont
# ignorés par git (.gitignore) : ils sont attachés à la GitHub Release.
#
# Comportement fail-close (set -euo pipefail) : toute erreur interrompt le
# packaging → en release, release-it avorte, aucune publication invalide.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"

read_cargo_version() {
    # Première ligne `version = "x.y.z"` du fichier (workspace.package).
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1
}

HORMOS_VERSION="${HORMOS_VERSION:-$(read_cargo_version)}"
HORMOS_VERSION="${HORMOS_VERSION#v}"
HORMOS_TARGET_LABEL="${HORMOS_TARGET_LABEL:-x86_64-unknown-linux-gnu}"
HORMOS_BINARY_PATH="${HORMOS_BINARY_PATH:-target/release/hormos}"

if [ -z "${HORMOS_VERSION}" ]; then
    echo "Erreur : HORMOS_VERSION introuvable (Cargo.toml ?)." >&2
    exit 1
fi
if [ ! -x "${HORMOS_BINARY_PATH}" ]; then
    echo "Erreur : binaire introuvable : ${HORMOS_BINARY_PATH}" >&2
    echo "Compilez d'abord la cible voulue (cargo build --release [--target ...])." >&2
    exit 1
fi

STAGE="hormos-v${HORMOS_VERSION}-${HORMOS_TARGET_LABEL}"
ARCHIVE="${STAGE}.tar.gz"

rm -rf "${STAGE}" "${ARCHIVE}" "${ARCHIVE}.sha256"

mkdir -p "${STAGE}"
install -m 0755 "${HORMOS_BINARY_PATH}" "${STAGE}/hormos"
install -m 0644 README.md "${STAGE}/README.md"
install -m 0644 LICENSE "${STAGE}/LICENSE"

# Archive REPRODUCTIBLE : même source + même SOURCE_DATE_EPOCH => octets
# identiques (permet la comparaison SHA-256 stricte des payloads en recovery).
# SOURCE_DATE_EPOCH est dérivé du commit de release exact (HORMOS_RELEASE_SHA),
# sinon du HEAD courant (qui, en CI, EST le commit de release après checkout).
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    _sha="${HORMOS_RELEASE_SHA:-HEAD}"
    SOURCE_DATE_EPOCH="$(git show -s --format=%ct "${_sha}" 2>/dev/null || true)"
fi
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    echo "Erreur : SOURCE_DATE_EPOCH introuvable (commit de release requis)." >&2
    exit 1
fi
export SOURCE_DATE_EPOCH

# tar GNU normalisé : ordre par nom, mtime figé, uid/gid/owner/group = 0.
# gzip -n : aucun nom/horodatage dans l'en-tête gzip.
tar --sort=name \
    --mtime="@${SOURCE_DATE_EPOCH}" \
    --owner=0 --group=0 --numeric-owner \
    --format=gnu \
    -cf - "${STAGE}" \
    | gzip -n > "${ARCHIVE}"

sha256sum "${ARCHIVE}" > "${ARCHIVE}.sha256"

# Vérifie immédiatement le checksum généré (corruption/troncature = échec ici).
sha256sum -c "${ARCHIVE}.sha256"

rm -rf "${STAGE}"

echo "Archive créée : ${ARCHIVE} (SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH})"
cat "${ARCHIVE}.sha256"

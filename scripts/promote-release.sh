#!/usr/bin/env bash
#
# Étape finale de la release d'Hormos (spec §5, §8, §14) : PROMOTION, exécutée
# UNIQUEMENT après validation complète des artefacts.
#
# Ordre : re-vérifier main == base testée → fast-forward main vers le commit de
# release C → créer le tag FINAL vX.Y.Z vers C. Les tags `v*` étant immuables,
# leur création est volontairement l'étape la plus tardive.
#
# Anti-TOCTOU (§2) : si `main` a avancé depuis le run, fail-close, aucune
# promotion, aucun tag, aucun force-push.
#
# Idempotence (§7, §8) : un rerun après promotion partielle reprend sans jamais
# réécrire/déplacer un tag existant.
#
# Variables d'environnement :
#   GH_TOKEN                 jeton de l'App (obligatoire).
#   REPO                     owner/repo. Défaut : GITHUB_REPOSITORY.
#   HORMOS_VERSION           version SemVer sans 'v'. Défaut : Cargo.toml.
#   HORMOS_RELEASE_BASE_SHA  base attendue (= github.sha). Obligatoire.
#   HORMOS_RELEASE_SHA       commit de release C. Obligatoire.
#
# Fail-close : set -euo pipefail.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=lib/release-lib.sh
. "${SCRIPT_DIR}/lib/release-lib.sh"

read_cargo_version() {
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1
}

REPO="${REPO:-${GITHUB_REPOSITORY:-Vesperis-group/hormos}}"
HORMOS_VERSION="${HORMOS_VERSION:-$(read_cargo_version)}"
HORMOS_VERSION="${HORMOS_VERSION#v}"
TAG="v${HORMOS_VERSION}"
BASE_SHA="${HORMOS_RELEASE_BASE_SHA:-}"
RELEASE_SHA="${HORMOS_RELEASE_SHA:-}"

[ -n "${GH_TOKEN:-}" ] || { echo "Erreur : GH_TOKEN requis." >&2; exit 1; }
[ -n "${BASE_SHA}" ] || { echo "Erreur : HORMOS_RELEASE_BASE_SHA requis." >&2; exit 1; }
[ -n "${RELEASE_SHA}" ] || { echo "Erreur : HORMOS_RELEASE_SHA requis." >&2; exit 1; }
for tool in gh jq; do
    command -v "${tool}" >/dev/null 2>&1 || { echo "Erreur : ${tool} requis." >&2; exit 1; }
done

# 0. Re-vérifier l'identité du commit de release (parent == base, Verified).
parent="$(gh api "repos/${REPO}/git/commits/${RELEASE_SHA}" --jq '.parents[0].sha // ""')"
verified="$(gh api "repos/${REPO}/commits/${RELEASE_SHA}" --jq '.commit.verification.verified')"
reason="$(gh api "repos/${REPO}/commits/${RELEASE_SHA}" --jq '.commit.verification.reason')"
[ "${parent}" = "${BASE_SHA}" ] || { echo "Erreur : parent(${RELEASE_SHA})=${parent} != base ${BASE_SHA}." >&2; exit 1; }
{ [ "${verified}" = "true" ] && [ "${reason}" = "valid" ]; } || { echo "Erreur : commit de release non Verified." >&2; exit 1; }

# 1. Anti-TOCTOU : état actuel de main.
MAIN_SHA="$(gh api "repos/${REPO}/git/refs/heads/main" --jq '.object.sha')"
if [ "${MAIN_SHA}" = "${RELEASE_SHA}" ]; then
    echo "main est déjà au commit de release (rerun après promotion) : ${RELEASE_SHA}"
elif rl_remote_base_ok "${BASE_SHA}" "${MAIN_SHA}"; then
    echo "main == base attendue (${BASE_SHA}). Promotion fast-forward vers ${RELEASE_SHA}."
    gh api -X PATCH "repos/${REPO}/git/refs/heads/main" \
        -F sha="${RELEASE_SHA}" -F force=false --jq '.object.sha' >/dev/null
    echo "main promu vers ${RELEASE_SHA}."
else
    # rl_remote_base_ok a déjà émis le message « main advanced during release ».
    exit 1
fi

# 2. Tag final vX.Y.Z -> C. Immuable : jamais déplacé.
resolve_tag_commit() {
    # Résout un tag (annoté ou léger) vers son commit cible ; vide si absent.
    local j otype osha
    j="$(gh api "repos/${REPO}/git/refs/tags/${TAG}" 2>/dev/null)" || { printf ''; return 0; }
    otype="$(jq -r '.object.type' <<<"${j}")"
    osha="$(jq -r '.object.sha' <<<"${j}")"
    if [ "${otype}" = "tag" ]; then
        gh api "repos/${REPO}/git/tags/${osha}" --jq '.object.sha'
    else
        printf '%s' "${osha}"
    fi
}

existing_tag_commit="$(resolve_tag_commit)"
if [ -n "${existing_tag_commit}" ]; then
    echo "Tag ${TAG} déjà présent -> ${existing_tag_commit}"
    rl_tag_points_ok "${existing_tag_commit}" "${RELEASE_SHA}"
    echo "Tag ${TAG} cohérent (rerun sûr), inchangé."
else
    tag_obj="$(jq -n --arg t "${TAG}" --arg m "Release ${TAG}" --arg o "${RELEASE_SHA}" \
        '{tag:$t, message:$m, object:$o, type:"commit"}' \
        | gh api -X POST "repos/${REPO}/git/tags" --input - --jq '.sha')"
    gh api -X POST "repos/${REPO}/git/refs" \
        -f ref="refs/tags/${TAG}" -f sha="${tag_obj}" --jq '.ref' >/dev/null
    echo "Tag final ${TAG} créé -> ${RELEASE_SHA}"
fi

# 3. Vérification post-promotion (§14) : le tag résout bien vers C.
final_tag_commit="$(resolve_tag_commit)"
rl_tag_points_ok "${final_tag_commit}" "${RELEASE_SHA}"
echo "OK : main == ${RELEASE_SHA}, tag ${TAG} -> ${RELEASE_SHA}."

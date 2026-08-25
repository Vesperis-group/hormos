#!/usr/bin/env bash
#
# Étape 1 de la release d'Hormos : crée le **commit de release Verified** via la
# Git Data API (jeton de la GitHub App), sur un **parent explicite** (la base
# testée par quality/audit), puis publie ce commit sur une **ref de staging**
# non-`v*`, non-`main`. Ne touche NI à `main` NI aux tags : la promotion est
# tardive (voir scripts/promote-release.sh) après validation des artefacts.
#
# Anti-TOCTOU : le parent du commit est HORMOS_RELEASE_BASE_SHA (= github.sha du
# run), pas un `main` flottant. Le HEAD local doit déjà être cette base.
#
# Verified : un commit créé par l'App via l'API est signé par GitHub (prouvé §8).
# Le script échoue si verification.verified != true ou reason != valid.
#
# Idempotence (§7) : la ref de staging est déterministe (version + base). Si elle
# existe déjà et pointe vers un commit d'identité EXACTE (parent, tree, version,
# Verified), on la réutilise ; sinon fail-close.
#
# Variables d'environnement :
#   GH_TOKEN                 jeton de l'App (obligatoire).
#   REPO                     owner/repo. Défaut : GITHUB_REPOSITORY.
#   HORMOS_VERSION           version SemVer sans 'v'. Défaut : Cargo.toml.
#   HORMOS_RELEASE_BASE_SHA  base attendue (= github.sha). Obligatoire en CI.
#
# Sorties (GITHUB_OUTPUT si défini + fichier .release-sha) :
#   release_sha, base_sha, tag, staging_ref
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

[ -n "${GH_TOKEN:-}" ] || { echo "Erreur : GH_TOKEN (jeton de l'App) requis." >&2; exit 1; }
[ -n "${HORMOS_VERSION}" ] || { echo "Erreur : HORMOS_VERSION introuvable." >&2; exit 1; }
[ -n "${BASE_SHA}" ] || { echo "Erreur : HORMOS_RELEASE_BASE_SHA requis (base testée)." >&2; exit 1; }
for tool in gh jq; do
    command -v "${tool}" >/dev/null 2>&1 || { echo "Erreur : ${tool} requis." >&2; exit 1; }
done

# 1. Le HEAD local doit être EXACTEMENT la base testée par quality/audit.
rl_base_sha_ok "${BASE_SHA}" "$(git rev-parse HEAD)"

# 2. Allowlist stricte du worktree : seuls Cargo.toml/Cargo.lock/CHANGELOG.md
#    peuvent avoir été modifiés par la préparation.
PORCELAIN="$(git status --porcelain)"
rl_worktree_allowlist_ok "${PORCELAIN}"

mapfile -t CHANGED < <(git status --porcelain -- Cargo.toml Cargo.lock CHANGELOG.md | sed 's/^...//')
[ "${#CHANGED[@]}" -gt 0 ] || { echo "Erreur : aucun fichier de release modifié." >&2; exit 1; }
echo "Fichiers du commit de release : ${CHANGED[*]}"

# 3. Arbre de base = arbre du commit BASE_SHA (parent explicite).
BASE_TREE="$(gh api "repos/${REPO}/git/commits/${BASE_SHA}" --jq '.tree.sha')"
echo "Base=${BASE_SHA} tree=${BASE_TREE}"

# 4. Un blob par fichier modifié, puis le nouvel arbre (base_tree + allowlist).
tree_items="[]"
for f in "${CHANGED[@]}"; do
    content_b64="$(base64 -w0 < "${f}")"
    blob_sha="$(jq -n --arg c "${content_b64}" '{content:$c, encoding:"base64"}' \
        | gh api -X POST "repos/${REPO}/git/blobs" --input - --jq '.sha')"
    tree_items="$(jq -c --arg p "${f}" --arg s "${blob_sha}" \
        '. + [{path:$p, mode:"100644", type:"blob", sha:$s}]' <<<"${tree_items}")"
    echo "  blob ${f} -> ${blob_sha}"
done
NEW_TREE="$(jq -n --arg bt "${BASE_TREE}" --argjson items "${tree_items}" \
    '{base_tree:$bt, tree:$items}' \
    | gh api -X POST "repos/${REPO}/git/trees" --input - --jq '.sha')"
echo "Nouvel arbre : ${NEW_TREE}"

COMMIT_MSG="chore: release ${TAG} [skip ci]"
STAGING_REF="release-staging/${TAG}-${BASE_SHA:0:12}"

# 5. Recovery (§3, §4) : réutiliser un commit de release existant SEULEMENT si
#    son identité correspond EXACTEMENT (parent, tree, message, Verified), afin
#    de ne JAMAIS fabriquer un second commit C2 après perte de la ref de staging.
verify_commit_identity() {
    local sha="$1" j parent tree msg ver reason
    j="$(gh api "repos/${REPO}/git/commits/${sha}")" || return 1
    parent="$(jq -r '.parents[0].sha // ""' <<<"${j}")"
    tree="$(jq -r '.tree.sha' <<<"${j}")"
    msg="$(jq -r '.message' <<<"${j}")"
    ver="$(gh api "repos/${REPO}/commits/${sha}" --jq '.commit.verification.verified')"
    reason="$(gh api "repos/${REPO}/commits/${sha}" --jq '.commit.verification.reason')"
    rl_commit_identity_ok "${parent}" "${tree}" "${msg}" "${ver}" "${reason}" \
        "${BASE_SHA}" "${NEW_TREE}" "${COMMIT_MSG}"
}

resolve_tag_commit() {
    # Résout refs/tags/<TAG> (annoté ou léger) vers son commit ; vide si absent.
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

ensure_staging_ref() {
    # (Re)crée la ref de staging vers le commit fourni si elle n'existe pas.
    local sha="$1" cur
    if cur="$(gh api "repos/${REPO}/git/refs/heads/${STAGING_REF}" --jq '.object.sha' 2>/dev/null)"; then
        [ "${cur}" = "${sha}" ] || { echo "Erreur : staging ${STAGING_REF} -> ${cur} != ${sha}." >&2; return 1; }
    else
        gh api -X POST "repos/${REPO}/git/refs" \
            -f ref="refs/heads/${STAGING_REF}" -f sha="${sha}" --jq '.ref' >/dev/null
        echo "Ref de staging (re)créée : ${STAGING_REF} -> ${sha}"
    fi
}

COMMIT_SHA=""
REUSED_FROM=""

# 5a. Ref de staging présente.
if existing="$(gh api "repos/${REPO}/git/refs/heads/${STAGING_REF}" --jq '.object.sha' 2>/dev/null)"; then
    echo "Ref de staging existante : ${STAGING_REF} -> ${existing}"
    if verify_commit_identity "${existing}"; then
        COMMIT_SHA="${existing}"; REUSED_FROM="staging"
    else
        echo "Erreur : staging existant d'identité différente. Fail-close." >&2
        exit 1
    fi
fi

# 5b. Sinon, tag final déjà présent (release partiellement/complètement promue).
if [ -z "${COMMIT_SHA}" ]; then
    tag_commit="$(resolve_tag_commit)"
    if [ -n "${tag_commit}" ]; then
        echo "Tag ${TAG} présent -> ${tag_commit} ; vérification d'identité."
        if verify_commit_identity "${tag_commit}"; then
            COMMIT_SHA="${tag_commit}"; REUSED_FROM="tag"
        else
            echo "Erreur : tag ${TAG} pointe un commit d'identité incompatible. Fail-close." >&2
            exit 1
        fi
    fi
fi

# 5c. Sinon, main a déjà été promu vers le commit de release attendu.
if [ -z "${COMMIT_SHA}" ]; then
    main_sha="$(gh api "repos/${REPO}/git/refs/heads/main" --jq '.object.sha')"
    if [ "${main_sha}" != "${BASE_SHA}" ] && verify_commit_identity "${main_sha}"; then
        echo "main déjà promu vers le commit de release attendu : ${main_sha}"
        COMMIT_SHA="${main_sha}"; REUSED_FROM="main"
    fi
fi

if [ -n "${COMMIT_SHA}" ]; then
    echo "Réutilisation du commit de release existant (source=${REUSED_FROM}, rerun sûr)."
    ensure_staging_ref "${COMMIT_SHA}"
else
    # 6. Aucun commit compatible : création via l'API (signé par GitHub → Verified).
    #    Aucun author/committer fourni : GitHub utilise l'identité de l'App.
    COMMIT_SHA="$(jq -n --arg m "${COMMIT_MSG}" --arg t "${NEW_TREE}" --arg p "${BASE_SHA}" \
        '{message:$m, tree:$t, parents:[$p]}' \
        | gh api -X POST "repos/${REPO}/git/commits" --input - --jq '.sha')"
    echo "Commit de release : ${COMMIT_SHA}"

    # 7. Fail-close : parent, tree et Verified doivent correspondre exactement.
    verify_commit_identity "${COMMIT_SHA}" || { echo "Erreur : identité du commit de release invalide." >&2; exit 1; }

    # 8. Publie sur la ref de staging (jamais main, jamais un tag v*).
    gh api -X POST "repos/${REPO}/git/refs" \
        -f ref="refs/heads/${STAGING_REF}" -f sha="${COMMIT_SHA}" --jq '.ref' >/dev/null
    echo "Ref de staging créée : ${STAGING_REF} -> ${COMMIT_SHA}"
fi

echo "Commit de release Verified : ${COMMIT_SHA} (parent ${BASE_SHA})"

# 9. Sorties.
printf '%s' "${COMMIT_SHA}" > .release-sha
if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "release_sha=${COMMIT_SHA}"
        echo "base_sha=${BASE_SHA}"
        echo "tag=${TAG}"
        echo "staging_ref=${STAGING_REF}"
    } >> "${GITHUB_OUTPUT}"
fi

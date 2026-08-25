#!/usr/bin/env bash
#
# Bibliothèque de fonctions PURES et testables pour la release d'Hormos.
#
# Aucune de ces fonctions n'appelle GitHub : les entrées (SHA, sorties de
# commandes) sont injectées en arguments, ce qui les rend unit-testables sans
# jeton ni réseau (voir scripts/tests/release-invariants.sh).
#
# Convention : chaque fonction renvoie 0 (OK) ou 1 (fail-close) et écrit un
# message explicite sur stderr en cas d'échec.

# Fichiers tracked que la préparation de release est autorisée à modifier.
# Toute autre modification tracked, ou tout untracked inattendu (non ignoré),
# doit provoquer un fail-close.
RL_ALLOWLIST=("Cargo.toml" "Cargo.lock" "CHANGELOG.md")

# rl_in_allowlist <path> : 0 si le chemin est dans l'allowlist, 1 sinon.
rl_in_allowlist() {
    local p="$1" a
    for a in "${RL_ALLOWLIST[@]}"; do
        [ "${p}" = "${a}" ] && return 0
    done
    return 1
}

# rl_base_sha_ok <expected> <actual> : le HEAD local doit être la base attendue.
rl_base_sha_ok() {
    local expected="$1" actual="$2"
    if [ -z "${expected}" ]; then
        echo "rl_base_sha_ok: base attendue vide (HORMOS_RELEASE_BASE_SHA manquant)" >&2
        return 1
    fi
    if [ "${expected}" != "${actual}" ]; then
        echo "rl_base_sha_ok: HEAD=${actual} != base attendue=${expected}" >&2
        return 1
    fi
    return 0
}

# rl_remote_base_ok <expected> <remote_main_sha> : main distant doit encore être
# la base attendue avant promotion (anti-TOCTOU).
rl_remote_base_ok() {
    local expected="$1" remote="$2"
    if [ "${expected}" != "${remote}" ]; then
        echo "main advanced during release; refusing to promote stale release (main=${remote} expected=${expected})" >&2
        return 1
    fi
    return 0
}

# rl_worktree_allowlist_ok <porcelain_text> : refuse toute modification tracked
# hors allowlist et tout untracked inattendu. `git status --porcelain` exclut
# déjà les fichiers ignorés (target/, node_modules/, artefacts hormos-v*).
rl_worktree_allowlist_ok() {
    local porcelain="$1" line path rc=0
    # Lecture ligne par ligne ; format porcelain "XY path" ou "XY old -> new".
    while IFS= read -r line; do
        [ -z "${line}" ] && continue
        path="${line:3}"
        # Renommage : ne garder que la cible.
        case "${path}" in
            *" -> "*) path="${path##* -> }" ;;
        esac
        # Retire d'éventuels guillemets ajoutés par git pour les chemins spéciaux.
        path="${path%\"}"
        path="${path#\"}"
        if ! rl_in_allowlist "${path}"; then
            echo "rl_worktree_allowlist_ok: modification interdite hors allowlist : ${path}" >&2
            rc=1
        fi
    done <<EOF
${porcelain}
EOF
    return "${rc}"
}

# rl_should_release_from_output <release_it_output> : imprime "true" ou "false".
# "No new version to release" (docs:/chore:/ci:/test: sans bump) => pas de release.
rl_should_release_from_output() {
    local out="$1"
    if printf '%s' "${out}" | grep -qiE 'No new version to release'; then
        printf 'false'
    else
        printf 'true'
    fi
}

# rl_tag_points_ok <resolved_tag_commit_sha> <expected_sha> : un tag existant ne
# doit JAMAIS être déplacé. 0 si résolu == attendu (reprise contrôlée possible),
# 1 sinon (fail-close).
rl_tag_points_ok() {
    local resolved="$1" expected="$2"
    if [ -z "${resolved}" ]; then
        echo "rl_tag_points_ok: SHA de tag résolu vide" >&2
        return 1
    fi
    if [ "${resolved}" != "${expected}" ]; then
        echo "rl_tag_points_ok: tag pointe ${resolved} != release SHA attendu ${expected}" >&2
        return 1
    fi
    return 0
}

# rl_commit_identity_ok <parent> <tree> <msg> <verified> <reason> \
#                       <exp_parent> <exp_tree> <exp_msg>
# Décide PUREMENT si un commit candidat peut être réutilisé pour la recovery :
# parent, tree et message doivent correspondre EXACTEMENT à l'attendu, et le
# commit doit être Verified (reason=valid). Ne jamais accepter sur le seul
# message/version.
rl_commit_identity_ok() {
    local parent="$1" tree="$2" msg="$3" verified="$4" reason="$5" \
          exp_parent="$6" exp_tree="$7" exp_msg="$8"
    if [ "${parent}" != "${exp_parent}" ]; then
        echo "rl_commit_identity_ok: parent ${parent} != ${exp_parent}" >&2; return 1
    fi
    if [ "${tree}" != "${exp_tree}" ]; then
        echo "rl_commit_identity_ok: tree ${tree} != ${exp_tree}" >&2; return 1
    fi
    if [ "${msg}" != "${exp_msg}" ]; then
        echo "rl_commit_identity_ok: message inattendu" >&2; return 1
    fi
    if [ "${verified}" != "true" ] || [ "${reason}" != "valid" ]; then
        echo "rl_commit_identity_ok: commit non Verified (${verified}/${reason})" >&2; return 1
    fi
    return 0
}

# rl_sha_match <local_sha> <remote_sha> : intégrité par contenu (SHA-256). Refuse
# la réutilisation d'un asset distant dont le contenu diffère du local.
rl_sha_match() {
    local a="$1" b="$2"
    if [ -z "${a}" ] || [ -z "${b}" ]; then
        echo "rl_sha_match: empreinte vide (local='${a}' remote='${b}')" >&2; return 1
    fi
    if [ "${a}" != "${b}" ]; then
        echo "rl_sha_match: mismatch local=${a} remote=${b}" >&2; return 1
    fi
    return 0
}

# rl_assets_no_unexpected <remote_names_nl> <expected_names_nl> : refuse tout
# asset distant hors de l'ensemble attendu (ex. hormos-v*-backdoor.tar.gz).
# Les deux arguments sont des listes de noms séparés par des sauts de ligne.
rl_assets_no_unexpected() {
    local remote="$1" expected="$2" name rc=0
    while IFS= read -r name; do
        [ -z "${name}" ] && continue
        if ! printf '%s\n' "${expected}" | grep -qxF "${name}"; then
            echo "rl_assets_no_unexpected: asset distant inattendu : ${name}" >&2
            rc=1
        fi
    done <<EOF
${remote}
EOF
    return "${rc}"
}

# rl_asset_class <name> : classe un asset de release. La politique de recovery
# DIFFÈRE selon la classe :
#   PAYLOAD    -> contenu reproductible : comparaison SHA-256 stricte.
#   SIGNATURE  -> bundle cosign sign-blob : vérification CRYPTO (jamais SHA).
#   PROVENANCE -> bundle cosign attest-blob : vérif CRYPTO + claims (jamais SHA).
#   INTOTO     -> agrégat DSSE : dérivé des bundles provenance canoniques.
# Écrit la classe sur stdout.
rl_asset_class() {
    local n="$1"
    case "${n}" in
        *.provenance.sigstore.json) printf 'PROVENANCE' ;;
        *.sigstore.json)            printf 'SIGNATURE' ;;
        *.intoto.jsonl)             printf 'INTOTO' ;;
        *)                          printf 'PAYLOAD' ;;
    esac
}

# rl_provenance_claims_ok <predicate_json> <expected_gitcommit> <expected_ref>
# Vérifie PUREMENT les claims critiques d'un prédicat de provenance SLSA v1 :
# le gitCommit attesté == release SHA et la ref == refs/tags/vX.Y.Z. N'exige PAS
# que invocationId / run_id / run_attempt correspondent au rerun courant (une
# preuve d'une tentative précédente reste valide pour le même SHA/ref).
rl_provenance_claims_ok() {
    local pred="$1" exp_commit="$2" exp_ref="$3" commit ref
    commit="$(printf '%s' "${pred}" \
        | jq -r '.buildDefinition.resolvedDependencies[0].digest.gitCommit // ""' 2>/dev/null)"
    ref="$(printf '%s' "${pred}" \
        | jq -r '.buildDefinition.externalParameters.workflow.ref // ""' 2>/dev/null)"
    if [ "${commit}" != "${exp_commit}" ]; then
        echo "rl_provenance_claims_ok: gitCommit ${commit} != ${exp_commit}" >&2; return 1
    fi
    if [ "${ref}" != "${exp_ref}" ]; then
        echo "rl_provenance_claims_ok: ref ${ref} != ${exp_ref}" >&2; return 1
    fi
    return 0
}

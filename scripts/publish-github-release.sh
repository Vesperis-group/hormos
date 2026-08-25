#!/usr/bin/env bash
#
# Publie/complète la GitHub Release d'Hormos de façon IDEMPOTENTE, avec une
# politique de recovery DIFFÉRENCIÉE par classe d'asset (spec §3, §9-18) :
#
#   PAYLOAD reproductible (archives, .sha256, SBOM, checksums)
#     -> comparaison SHA-256 stricte (même release SHA => octets identiques).
#        Mismatch = anomalie supply-chain => fail-close.
#
#   SIGNATURE / PROVENANCE Sigstore (*.sigstore.json, *.provenance.sigstore.json)
#     -> preuves cryptographiques NON déterministes : vérifiées par cosign
#        (identité EXACTE release.yml@main + issuer + claims), JAMAIS comparées
#        octet-à-octet à une preuve fraîchement régénérée. Une preuve d'une
#        tentative antérieure reste valide pour le même release SHA / ref.
#
#   INTOTO agrégat (*.provenance.intoto.jsonl)
#     -> reconstruit à partir des bundles provenance CANONIQUES réellement
#        présents dans la Release, puis comparé octet-à-octet (inputs figés).
#
# Jamais de --clobber, jamais de suppression/remplacement d'une evidence publiée.
#
# Variables : GH_TOKEN (App), REPO, HORMOS_VERSION, HORMOS_RELEASE_SHA.
# Fail-close : set -euo pipefail.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${PROJECT_DIR}"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=lib/release-lib.sh
. "${SCRIPT_DIR}/lib/release-lib.sh"

read_cargo_version() { sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1; }

REPO="${REPO:-${GITHUB_REPOSITORY:-Vesperis-group/hormos}}"
HORMOS_VERSION="${HORMOS_VERSION:-$(read_cargo_version)}"
HORMOS_VERSION="${HORMOS_VERSION#v}"
V="${HORMOS_VERSION}"
TAG="v${V}"
RELEASE_SHA="${HORMOS_RELEASE_SHA:-}"
RELEASE_REF="refs/tags/${TAG}"

[ -n "${GH_TOKEN:-}" ] || { echo "Erreur : GH_TOKEN requis." >&2; exit 1; }
[ -n "${RELEASE_SHA}" ] || { echo "Erreur : HORMOS_RELEASE_SHA requis." >&2; exit 1; }
for tool in gh jq cosign; do
    command -v "${tool}" >/dev/null 2>&1 || { echo "Erreur : ${tool} requis." >&2; exit 1; }
done

IDENTITY="$(HORMOS_SIGN_PRINT_IDENTITY=1 bash "${SCRIPT_DIR}/sign-release.sh")"
ISSUER="${HORMOS_SIGN_OIDC_ISSUER:-https://token.actions.githubusercontent.com}"

GLIBC="hormos-v${V}-x86_64-unknown-linux-gnu-glibc2.35.tar.gz"
MUSL="hormos-v${V}-x86_64-unknown-linux-musl.tar.gz"
SBOM="hormos-v${V}-sbom.cdx.json"
CHECKSUMS="hormos-v${V}-checksums.txt"
INTOTO="hormos-v${V}-provenance.intoto.jsonl"

# Ordre d'upload/recovery stable (spec §18) : A payloads, B checksums,
# C signatures, D provenance, E intoto.
PAYLOADS=(
    "${GLIBC}" "${GLIBC}.sha256"
    "${MUSL}" "${MUSL}.sha256"
    "${SBOM}" "${SBOM}.sha256"
    "${CHECKSUMS}"
)
SIGNED=("${GLIBC}" "${MUSL}" "${SBOM}" "${CHECKSUMS}")
SIG_BUNDLES=(); PROV_BUNDLES=()
for a in "${SIGNED[@]}"; do
    SIG_BUNDLES+=("${a}.sigstore.json")
    PROV_BUNDLES+=("${a}.provenance.sigstore.json")
done

# Ensemble EXACT attendu (pour le contrôle « aucun asset inattendu »).
EXPECTED=("${PAYLOADS[@]}" "${SIG_BUNDLES[@]}" "${PROV_BUNDLES[@]}" "${INTOTO}")
EXPECTED_LIST="$(printf '%s\n' "${EXPECTED[@]}")"

# Tous les artefacts locaux (déjà validés par validate-artifacts.sh) présents.
for a in "${EXPECTED[@]}"; do
    [ -s "${a}" ] || { echo "Erreur : asset local manquant/vide : ${a}" >&2; exit 1; }
done

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
local_sha256() { sha256sum "$1" | cut -d' ' -f1; }

resolve_tag_commit() {
    local j otype osha
    j="$(gh api "repos/${REPO}/git/refs/tags/${TAG}")"
    otype="$(jq -r '.object.type' <<<"${j}")"
    osha="$(jq -r '.object.sha' <<<"${j}")"
    if [ "${otype}" = "tag" ]; then
        gh api "repos/${REPO}/git/tags/${osha}" --jq '.object.sha'
    else
        printf '%s' "${osha}"
    fi
}

# download_asset <name> <destdir> : télécharge un asset de la Release ; 0 si ok.
download_asset() {
    local name="$1" dir="$2"
    gh release download "${TAG}" --repo "${REPO}" --pattern "${name}" --dir "${dir}" >/dev/null 2>&1 \
        && [ -s "${dir}/${name}" ]
}

# remote_has <name> : 0 si l'asset est présent dans la Release.
remote_has() { printf '%s\n' "${REMOTE_NAMES}" | grep -qxF "$1"; }

# verify_signature <payload> <bundle_path> : vraie vérification cosign sign-blob.
verify_signature() {
    cosign verify-blob \
        --bundle "$2" \
        --certificate-identity-regexp "${IDENTITY}" \
        --certificate-oidc-issuer "${ISSUER}" \
        "$1" >/dev/null 2>&1
}

# verify_provenance <payload> <bundle_path> : cosign verify-blob-attestation +
# contrôle des claims (gitCommit == RELEASE_SHA, ref == refs/tags/vX.Y.Z).
verify_provenance() {
    local payload="$1" bundle="$2" pred
    cosign verify-blob-attestation \
        --bundle "${bundle}" \
        --type slsaprovenance1 \
        --check-claims=true \
        --certificate-identity-regexp "${IDENTITY}" \
        --certificate-oidc-issuer "${ISSUER}" \
        "${payload}" >/dev/null 2>&1 || return 1
    # Extrait le prédicat depuis l'enveloppe DSSE du bundle.
    pred="$(jq -r '.dsseEnvelope.payload // .payload // empty' "${bundle}" 2>/dev/null \
        | base64 -d 2>/dev/null | jq '.predicate' 2>/dev/null)"
    [ -n "${pred}" ] || { echo "verify_provenance: prédicat illisible (${bundle})" >&2; return 1; }
    rl_provenance_claims_ok "${pred}" "${RELEASE_SHA}" "${RELEASE_REF}"
}

# --- Le tag doit exister et pointer vers le commit de release --------------
tag_commit="$(resolve_tag_commit)"
[ "${tag_commit}" = "${RELEASE_SHA}" ] || { echo "Erreur : tag ${TAG} -> ${tag_commit} != ${RELEASE_SHA}." >&2; exit 1; }

# --- Création de la Release si absente (sans asset au départ) --------------
if gh release view "${TAG}" --repo "${REPO}" >/dev/null 2>&1; then
    meta="$(gh release view "${TAG}" --repo "${REPO}" --json tagName,isDraft,isPrerelease)"
    [ "$(jq -r '.tagName' <<<"${meta}")" = "${TAG}" ] || { echo "Erreur : tagName inattendu." >&2; exit 1; }
    [ "$(jq -r '.isDraft' <<<"${meta}")" = "false" ] || { echo "Erreur : Release en draft." >&2; exit 1; }
    [ "$(jq -r '.isPrerelease' <<<"${meta}")" = "false" ] || { echo "Erreur : Release en prerelease (non prévu)." >&2; exit 1; }
    echo "Release ${TAG} existante : recovery différenciée par classe."
else
    echo "Création de la Release ${TAG} (sans asset)."
    gh release create "${TAG}" \
        --repo "${REPO}" \
        --title "hormos ${TAG}" \
        --notes "Release ${TAG}. Voir CHANGELOG.md. Artefacts signés (cosign keyless) avec provenance SLSA."
fi

REMOTE_NAMES="$(gh release view "${TAG}" --repo "${REPO}" --json assets --jq '.assets[].name')"
rl_assets_no_unexpected "${REMOTE_NAMES}" "${EXPECTED_LIST}"

# --- A/B. Payloads reproductibles : SHA-256 strict -------------------------
for a in "${PAYLOADS[@]}"; do
    if remote_has "${a}"; then
        dir="$(mktemp -d "${TMP}/pXXXX")"
        download_asset "${a}" "${dir}" || { echo "Erreur : téléchargement ${a}." >&2; exit 1; }
        rsha="$(local_sha256 "${dir}/${a}")"; lsha="$(local_sha256 "${a}")"
        if ! rl_sha_match "${lsha}" "${rsha}"; then
            echo "Reproducible release payload mismatch for the same release SHA" >&2
            echo "asset: ${a}" >&2
            echo "remote sha256: ${rsha}" >&2
            echo "local sha256: ${lsha}" >&2
            echo "refusing to overwrite immutable release evidence" >&2
            exit 1
        fi
        echo "  payload intègre (reuse) : ${a}"
    else
        echo "  payload upload : ${a}"
        gh release upload "${TAG}" --repo "${REPO}" "${a}"
        dir="$(mktemp -d "${TMP}/pXXXX")"
        download_asset "${a}" "${dir}" || { echo "Erreur : re-download ${a}." >&2; exit 1; }
        rl_sha_match "$(local_sha256 "${a}")" "$(local_sha256 "${dir}/${a}")" \
            || { echo "Erreur : upload de ${a} non vérifié (SHA)." >&2; exit 1; }
    fi
    REMOTE_NAMES="$(gh release view "${TAG}" --repo "${REPO}" --json assets --jq '.assets[].name')"
done

# --- C. Signatures : vérification cryptographique (jamais SHA) --------------
for a in "${SIGNED[@]}"; do
    b="${a}.sigstore.json"
    if remote_has "${b}"; then
        dir="$(mktemp -d "${TMP}/sXXXX")"
        download_asset "${b}" "${dir}" || { echo "Erreur : téléchargement ${b}." >&2; exit 1; }
        # Le payload local == payload distant (déjà prouvé par SHA ci-dessus).
        if verify_signature "${a}" "${dir}/${b}"; then
            echo "  signature valide (reuse) : ${b}"
        else
            echo "Erreur : signature distante invalide : ${b} (identité/issuer/payload)." >&2
            echo "refusing to overwrite immutable release evidence" >&2
            exit 1
        fi
    else
        echo "  signature upload : ${b}"
        gh release upload "${TAG}" --repo "${REPO}" "${b}"
        dir="$(mktemp -d "${TMP}/sXXXX")"
        download_asset "${b}" "${dir}" || { echo "Erreur : re-download ${b}." >&2; exit 1; }
        verify_signature "${a}" "${dir}/${b}" \
            || { echo "Erreur : signature ${b} non vérifiée après upload." >&2; exit 1; }
    fi
    REMOTE_NAMES="$(gh release view "${TAG}" --repo "${REPO}" --json assets --jq '.assets[].name')"
done

# --- D. Provenance : vérification cryptographique + claims (jamais SHA) -----
for a in "${SIGNED[@]}"; do
    b="${a}.provenance.sigstore.json"
    if remote_has "${b}"; then
        dir="$(mktemp -d "${TMP}/vXXXX")"
        download_asset "${b}" "${dir}" || { echo "Erreur : téléchargement ${b}." >&2; exit 1; }
        if verify_provenance "${a}" "${dir}/${b}"; then
            echo "  provenance valide (reuse) : ${b}"
        else
            echo "Erreur : provenance distante invalide : ${b} (crypto/claims)." >&2
            echo "refusing to overwrite immutable release evidence" >&2
            exit 1
        fi
    else
        echo "  provenance upload : ${b}"
        gh release upload "${TAG}" --repo "${REPO}" "${b}"
        dir="$(mktemp -d "${TMP}/vXXXX")"
        download_asset "${b}" "${dir}" || { echo "Erreur : re-download ${b}." >&2; exit 1; }
        verify_provenance "${a}" "${dir}/${b}" \
            || { echo "Erreur : provenance ${b} non vérifiée après upload." >&2; exit 1; }
    fi
    REMOTE_NAMES="$(gh release view "${TAG}" --repo "${REPO}" --json assets --jq '.assets[].name')"
done

# --- E. intoto : reconstruit depuis les bundles provenance CANONIQUES ------
# Les bundles réellement présents dans la Release (remote) sont l'evidence
# canonique. On reconstruit l'agrégat DSSE depuis EUX (ordre SIGNED figé), ce qui
# rend la comparaison octet-à-octet pertinente.
PROVDIR="$(mktemp -d "${TMP}/provXXXX")"
CANON_INTOTO="${TMP}/canon.intoto.jsonl"
: > "${CANON_INTOTO}"
for a in "${SIGNED[@]}"; do
    b="${a}.provenance.sigstore.json"
    download_asset "${b}" "${PROVDIR}" || { echo "Erreur : téléchargement canonique ${b}." >&2; exit 1; }
    jq -c '.dsseEnvelope' "${PROVDIR}/${b}" >> "${CANON_INTOTO}"
done
[ -s "${CANON_INTOTO}" ] || { echo "Erreur : agrégat intoto canonique vide." >&2; exit 1; }

if remote_has "${INTOTO}"; then
    dir="$(mktemp -d "${TMP}/iXXXX")"
    download_asset "${INTOTO}" "${dir}" || { echo "Erreur : téléchargement ${INTOTO}." >&2; exit 1; }
    if rl_sha_match "$(local_sha256 "${CANON_INTOTO}")" "$(local_sha256 "${dir}/${INTOTO}")"; then
        echo "  intoto cohérent (reuse) : ${INTOTO}"
    else
        echo "Erreur : ${INTOTO} distant incohérent avec les bundles provenance canoniques." >&2
        echo "refusing to overwrite immutable release evidence" >&2
        exit 1
    fi
else
    echo "  intoto upload (reconstruit depuis bundles canoniques) : ${INTOTO}"
    cp "${CANON_INTOTO}" "${INTOTO}"
    gh release upload "${TAG}" --repo "${REPO}" "${INTOTO}"
    dir="$(mktemp -d "${TMP}/iXXXX")"
    download_asset "${INTOTO}" "${dir}" || { echo "Erreur : re-download ${INTOTO}." >&2; exit 1; }
    rl_sha_match "$(local_sha256 "${CANON_INTOTO}")" "$(local_sha256 "${dir}/${INTOTO}")" \
        || { echo "Erreur : ${INTOTO} non vérifié après upload." >&2; exit 1; }
fi

# --- Vérification finale : ensemble EXACT + provenance intoto non vide ------
echo "== vérification post-release =="
final_names="$(gh release view "${TAG}" --repo "${REPO}" --json assets --jq '.assets[] | select(.size > 0) | .name')"
rl_assets_no_unexpected "${final_names}" "${EXPECTED_LIST}"
for a in "${EXPECTED[@]}"; do
    printf '%s\n' "${final_names}" | grep -qxF "${a}" \
        || { echo "Erreur : asset absent/vide dans la Release : ${a}" >&2; exit 1; }
done

echo "OK : Release ${TAG} publiée et vérifiée (tag -> ${RELEASE_SHA}, ${#EXPECTED[@]} assets)."
#!/usr/bin/env bash
#
# Tests des invariants de release d'Hormos (spec §18). Pur bash, sans mocks
# lourds ni intégration GitHub : on teste les fonctions pures de
# scripts/lib/release-lib.sh et l'identité Sigstore.
#
# Aucune release réelle, aucun tag v*.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../lib/release-lib.sh
. "${SCRIPT_DIR}/../lib/release-lib.sh"

pass=0
fail=0

ok() { echo "ok   - $1"; pass=$((pass + 1)); }
ko() { echo "FAIL - $1"; fail=$((fail + 1)); }

# expect_ok <desc> : la commande suivante (déjà exécutée) doit avoir réussi.
check() {
    local desc="$1" rc="$2" want="$3"
    if [ "${rc}" = "${want}" ]; then ok "${desc}"; else ko "${desc} (rc=${rc}, want=${want})"; fi
}

# expect_eq <desc> <got> <want>
expect_eq() {
    local desc="$1" got="$2" want="$3"
    if [ "${got}" = "${want}" ]; then ok "${desc}"; else ko "${desc} (got=${got}, want=${want})"; fi
}

BASE="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
OTHER="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
RELEASE="cccccccccccccccccccccccccccccccccccccccc"

echo "== SHA =="
rl_base_sha_ok "${BASE}" "${BASE}" 2>/dev/null; check "1. HEAD == base attendue -> OK" "$?" "0"
rl_base_sha_ok "${BASE}" "${OTHER}" 2>/dev/null; check "2. HEAD != base attendue -> FAIL" "$?" "1"
rl_remote_base_ok "${BASE}" "${BASE}" 2>/dev/null; check "3. main distant == base -> OK" "$?" "0"
rl_remote_base_ok "${BASE}" "${OTHER}" 2>/dev/null; check "4. main distant avance -> FAIL sans promotion" "$?" "1"

echo "== release / no-op =="
noop_out="$(printf '🚀 release\nNo new version to release\n🏁 Done')"
rel_out="$(printf '🚀 release\nLet'\''s release hormos (0.0.0...0.1.0)\n')"
expect_eq "5. historique non releasable -> should_release=false" "$(rl_should_release_from_output "${noop_out}")" "false"
expect_eq "6. historique releasable -> should_release=true" "$(rl_should_release_from_output "${rel_out}")" "true"

echo "== worktree allowlist =="
rl_worktree_allowlist_ok "$(printf ' M Cargo.toml\n M Cargo.lock\n M CHANGELOG.md')" 2>/dev/null
check "7. uniquement allowlist modifiée -> OK" "$?" "0"
rl_worktree_allowlist_ok "$(printf ' M Cargo.toml\n M src/main.rs')" 2>/dev/null
check "8. autre fichier tracked modifié -> FAIL" "$?" "1"
rl_worktree_allowlist_ok "$(printf ' M Cargo.toml\n?? evil.sh')" 2>/dev/null
check "9. untracked inattendu -> FAIL" "$?" "1"

echo "== tag / recovery =="
# 10. tag absent => géré par l'appelant (résolu vide) : ici on vérifie le refus du vide.
rl_tag_points_ok "" "${RELEASE}" 2>/dev/null; check "10. tag résolu vide -> FAIL (traité par l'appelant comme absent)" "$?" "1"
rl_tag_points_ok "${RELEASE}" "${RELEASE}" 2>/dev/null; check "11. tag existant -> même release SHA -> reprise OK" "$?" "0"
rl_tag_points_ok "${OTHER}" "${RELEASE}" 2>/dev/null; check "12. tag existant -> autre SHA -> FAIL" "$?" "1"

echo "== Sigstore =="
if bash "${SCRIPT_DIR}/../test-sign-identity.sh" >/dev/null 2>&1; then
    ok "13-15. identité Sigstore (release.yml@main accepté, autres refusés)"
else
    ko "13-15. identité Sigstore"
fi

echo "== recovery : identité du commit réutilisable =="
TREE="dddddddddddddddddddddddddddddddddddddddd"
OTREE="eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
MSG="chore: release v0.1.0 [skip ci]"
# parent/tree/message OK + Verified -> reuse
rl_commit_identity_ok "${BASE}" "${TREE}" "${MSG}" "true" "valid" "${BASE}" "${TREE}" "${MSG}" 2>/dev/null
check "16. commit candidat identité exacte + Verified -> reuse" "$?" "0"
# tree différent -> FAIL
rl_commit_identity_ok "${BASE}" "${OTREE}" "${MSG}" "true" "valid" "${BASE}" "${TREE}" "${MSG}" 2>/dev/null
check "17. commit candidat tree différent -> FAIL" "$?" "1"
# non Verified -> FAIL
rl_commit_identity_ok "${BASE}" "${TREE}" "${MSG}" "false" "unsigned" "${BASE}" "${TREE}" "${MSG}" 2>/dev/null
check "18. commit candidat non Verified -> FAIL" "$?" "1"

echo "== asset integrity =="
rl_sha_match "abc123" "abc123" 2>/dev/null; check "19. remote/local même SHA -> OK" "$?" "0"
rl_sha_match "abc123" "def456" 2>/dev/null; check "20. remote/local SHA différent -> FAIL" "$?" "1"
EXP="$(printf 'hormos-v1-a.tar.gz\nhormos-v1-b.tar.gz')"
rl_assets_no_unexpected "$(printf 'hormos-v1-a.tar.gz\nhormos-v1-b.tar.gz')" "${EXP}" 2>/dev/null
check "21. ensemble distant == attendu -> OK" "$?" "0"
rl_assets_no_unexpected "$(printf 'hormos-v1-a.tar.gz\nhormos-v1-backdoor.tar.gz')" "${EXP}" 2>/dev/null
check "22. asset distant inattendu -> FAIL" "$?" "1"

echo "== classification des assets =="
expect_eq "23. archive -> PAYLOAD" "$(rl_asset_class hormos-v1-x86_64-unknown-linux-musl.tar.gz)" "PAYLOAD"
expect_eq "24. .sha256 -> PAYLOAD" "$(rl_asset_class hormos-v1-x86_64-unknown-linux-musl.tar.gz.sha256)" "PAYLOAD"
expect_eq "25. SBOM -> PAYLOAD" "$(rl_asset_class hormos-v1-sbom.cdx.json)" "PAYLOAD"
expect_eq "26. signature -> SIGNATURE" "$(rl_asset_class hormos-v1-x86_64-unknown-linux-musl.tar.gz.sigstore.json)" "SIGNATURE"
expect_eq "27. provenance -> PROVENANCE" "$(rl_asset_class hormos-v1-x86_64-unknown-linux-musl.tar.gz.provenance.sigstore.json)" "PROVENANCE"
expect_eq "28. intoto -> INTOTO" "$(rl_asset_class hormos-v1-provenance.intoto.jsonl)" "INTOTO"

echo "== provenance claims (fixtures JSON — pas une vérif cosign réelle) =="
PRED="$(printf '{"buildDefinition":{"externalParameters":{"workflow":{"ref":"refs/tags/v0.1.0"}},"resolvedDependencies":[{"digest":{"gitCommit":"%s"}}]}}' "${RELEASE}")"
rl_provenance_claims_ok "${PRED}" "${RELEASE}" "refs/tags/v0.1.0" 2>/dev/null; check "29. claims gitCommit+ref exacts -> OK" "$?" "0"
rl_provenance_claims_ok "${PRED}" "${OTHER}" "refs/tags/v0.1.0" 2>/dev/null; check "30. gitCommit différent -> FAIL" "$?" "1"
rl_provenance_claims_ok "${PRED}" "${RELEASE}" "refs/heads/main" 2>/dev/null; check "31. ref différente -> FAIL" "$?" "1"
PRED2="$(printf '{"buildDefinition":{"externalParameters":{"workflow":{"ref":"refs/tags/v0.1.0"}},"resolvedDependencies":[{"digest":{"gitCommit":"%s"}}]},"runDetails":{"metadata":{"invocationId":"run-999-attempt-7"}}}' "${RELEASE}")"
rl_provenance_claims_ok "${PRED2}" "${RELEASE}" "refs/tags/v0.1.0" 2>/dev/null; check "32. invocationId différent, même SHA/ref -> OK" "$?" "0"

echo "== reproductibilité des archives (package-release.sh) =="
dummy="$(mktemp)"; printf 'dummy-binary-content' > "${dummy}"; chmod +x "${dummy}"
ARCH="${PROJECT_DIR}/hormos-v0.0.0-x86_64-rtest.tar.gz"
gen_arch() {
    HORMOS_VERSION=0.0.0 HORMOS_TARGET_LABEL=x86_64-rtest HORMOS_BINARY_PATH="${dummy}" \
        SOURCE_DATE_EPOCH=1700000000 bash "${SCRIPT_DIR}/../package-release.sh" >/dev/null 2>&1
    sha256sum "${ARCH}" | cut -d' ' -f1
}
A="$(gen_arch)"; B="$(gen_arch)"
expect_eq "33. archive même source+epoch -> même SHA" "${A}" "${B}"
printf 'more' >> "${dummy}"
C="$(gen_arch)"
if [ "${A}" != "${C}" ]; then ok "34. archive contenu modifié -> SHA différent"; else ko "34. archive contenu modifié -> SHA différent"; fi
rm -f "${ARCH}" "${ARCH}.sha256" "${dummy}"

echo "== SBOM : identité CycloneDX RFC 4122 déterministe =="
CANON="${SCRIPT_DIR}/../lib/sbom-canonicalize.py"
sbom_dir="$(mktemp -d)"

# Fixture CycloneDX minimale mais valide. $2/$3 = champs volatils (serialNumber
# aléatoire, timestamp) ; $4 = version d'un composant (contenu significatif).
sbom_fixture() {
    cat > "$1" <<EOF
{"bomFormat":"CycloneDX","specVersion":"1.5","version":1,"serialNumber":"$2","metadata":{"timestamp":"$3","component":{"type":"application","name":"hormos"}},"components":[{"type":"library","name":"serde","version":"$4","purl":"pkg:cargo/serde@$4","hashes":[{"alg":"SHA-256","content":"deadbeef"}]}]}
EOF
}

# Identité du serialNumber : "<version>:<RFC4122|OTHER>".
sbom_uuid_id() {
    python3 - "$1" <<'PY'
import json
import sys
import uuid

serial = json.load(open(sys.argv[1], encoding="utf-8"))["serialNumber"]
if not serial.startswith("urn:uuid:"):
    raise SystemExit("préfixe urn:uuid: manquant")
parsed = uuid.UUID(serial[len("urn:uuid:"):])
variant = "RFC4122" if parsed.variant == uuid.RFC_4122 else "OTHER"
print(f"{parsed.version}:{variant}")
PY
}

# Valide une valeur de serialNumber avec le validateur du module (fail-close).
# -B : pas de __pycache__ (le worktree doit rester propre).
sbom_accepts() {
    python3 -B - "${CANON}" "$1" <<'PY'
import importlib.util
import sys

spec = importlib.util.spec_from_file_location("sbom_canonicalize", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
try:
    module.assert_rfc4122_v5(sys.argv[2])
except module.SbomError:
    sys.exit(1)
sys.exit(0)
PY
}

sbom_serial() {
    python3 -c 'import json,sys;print(json.load(open(sys.argv[1],encoding="utf-8"))["serialNumber"])' "$1"
}

SB_A="${sbom_dir}/a.cdx.json"
SB_B="${sbom_dir}/b.cdx.json"
SB_C="${sbom_dir}/c.cdx.json"
sbom_fixture "${SB_A}" "urn:uuid:11111111-1111-4111-8111-111111111111" "2020-01-01T00:00:00Z" "1.0.0"
sbom_fixture "${SB_B}" "urn:uuid:22222222-2222-4222-9222-222222222222" "2024-06-06T12:34:56Z" "1.0.0"
sbom_fixture "${SB_C}" "urn:uuid:11111111-1111-4111-8111-111111111111" "2020-01-01T00:00:00Z" "1.0.1"
python3 "${CANON}" "${SB_A}" >/dev/null 2>&1
python3 "${CANON}" "${SB_B}" >/dev/null 2>&1
python3 "${CANON}" "${SB_C}" >/dev/null 2>&1

expect_eq "35. serialNumber généré -> UUID v5 variant RFC 4122" "$(sbom_uuid_id "${SB_A}")" "5:RFC4122"
sbom_accepts "urn:uuid:11111111-1111-4111-8111-111111111111"; check "36. UUID v4 -> refusé" "$?" "1"
# Ancien défaut : nibble de version forcé à 5 mais bits de variant non contraints.
sbom_accepts "urn:uuid:11111111-1111-5111-3111-111111111111"; check "37. v5 mais variant non RFC 4122 -> refusé" "$?" "1"
sbom_accepts "11111111-1111-5111-8111-111111111111"; check "38. sans préfixe urn:uuid: -> refusé" "$?" "1"
sbom_accepts "$(sbom_serial "${SB_A}")"; check "39. serialNumber généré -> accepté" "$?" "0"

serial_a="$(sbom_serial "${SB_A}")"
serial_b="$(sbom_serial "${SB_B}")"
serial_c="$(sbom_serial "${SB_C}")"
expect_eq "40. champs volatils différents -> même serialNumber" "${serial_a}" "${serial_b}"
expect_eq "41. champs volatils différents -> même SHA-256" \
    "$(sha256sum "${SB_A}" | cut -d' ' -f1)" "$(sha256sum "${SB_B}" | cut -d' ' -f1)"
if [ "${serial_a}" != "${serial_c}" ]; then
    ok "42. contenu modifié -> serialNumber différent"
else
    ko "42. contenu modifié -> serialNumber différent"
fi
expect_eq "43. timestamp neutralisé, composants préservés" \
    "$(python3 -c 'import json,sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
has_ts = "timestamp" in d.get("metadata", {})
print("%s|%d|%s" % (has_ts, len(d["components"]), d["components"][0]["purl"]))' "${SB_A}")" \
    "False|1|pkg:cargo/serde@1.0.0"
rm -rf "${sbom_dir}"

echo
echo "Résumé : ${pass} OK, ${fail} FAIL"
[ "${fail}" -eq 0 ] || exit 1
echo "Tous les invariants de release sont vérifiés."

#!/usr/bin/env bash
#
# Test de l'identité Sigstore attendue (spec §6).
#
# Vérifie que la regex d'identité keyless produite par scripts/sign-release.sh :
#   - accepte EXACTEMENT le workflow release.yml du dépôt hormos sur main ;
#   - refuse tout autre workflow, branche ou dépôt.
#
# N'exécute aucune signature ; interroge sign-release.sh en mode
# HORMOS_SIGN_PRINT_IDENTITY=1.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

regex="$(HORMOS_SIGN_PRINT_IDENTITY=1 bash "${SCRIPT_DIR}/sign-release.sh")"
echo "Identité attendue : ${regex}"

should_match=(
    "https://github.com/Vesperis-group/hormos/.github/workflows/release.yml@refs/heads/main"
)
should_reject=(
    "https://github.com/Vesperis-group/hormos/.github/workflows/ci.yml@refs/heads/main"
    "https://github.com/Vesperis-group/hormos/.github/workflows/release.yml@refs/heads/dev"
    "https://github.com/Vesperis-group/hormos/.github/workflows/release.yml@refs/tags/v0.1.0"
    "https://github.com/attacker/hormos/.github/workflows/release.yml@refs/heads/main"
    "https://github.com/Vesperis-group/hormos-evil/.github/workflows/release.yml@refs/heads/main"
)

fail=0

for id in "${should_match[@]}"; do
    if printf '%s' "${id}" | grep -Eq "${regex}"; then
        echo "OK   accepte : ${id}"
    else
        echo "FAIL devrait accepter : ${id}"; fail=1
    fi
done

for id in "${should_reject[@]}"; do
    if printf '%s' "${id}" | grep -Eq "${regex}"; then
        echo "FAIL devrait refuser : ${id}"; fail=1
    else
        echo "OK   refuse  : ${id}"
    fi
done

if [ "${fail}" -ne 0 ]; then
    echo "Test d'identité Sigstore : ÉCHEC" >&2
    exit 1
fi
echo "Test d'identité Sigstore : OK"

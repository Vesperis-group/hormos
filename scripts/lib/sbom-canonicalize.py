#!/usr/bin/env python3
"""Canonicalise un SBOM CycloneDX JSON pour le rendre byte-reproductible.

cargo-cyclonedx n'a pas de sortie déterministe : chaque génération produit un
`serialNumber` aléatoire et un `metadata.timestamp` différents. Empiriquement,
deux SBOM du même projet sont identiques une fois ces DEUX seuls champs
neutralisés. On ne retire donc jamais un composant, une version, un hash, un
PURL, une licence, une relation ou une propriété security-relevant.

Choix Hormos (ce n'est PAS une exigence CycloneDX) : le `serialNumber` est
*déterministe*, dérivé de la représentation canonique du SBOM, afin qu'un SBOM
reconstruit pour un même release SHA soit byte-reproductible — condition du
recovery par SHA-256 des payloads.

Le point normatif CycloneDX 1.5 est unique : si `serialNumber` est présent, il
DOIT être conforme à la RFC 4122. On génère donc un véritable **UUID v5**
(`uuid.uuid5`, bibliothèque standard : bits de version ET de variant corrects),
puis on le **revalide explicitement** (reparsable, version == 5, variant ==
RFC_4122). Fail-close sinon.

Usage : python3 sbom-canonicalize.py <sbom.json>
"""

from __future__ import annotations

import json
import sys
import uuid

# Namespace projet : lui-même un UUID v5 dérivé de l'URL du dépôt.
PROJECT_URL = "https://github.com/Vesperis-group/hormos"
PROJECT_NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, PROJECT_URL)

URN_PREFIX = "urn:uuid:"


class SbomError(Exception):
    """Erreur de canonicalisation : toujours fail-close."""


def canonical_json(doc):
    """Représentation canonique stable : clés triées, séparateurs fixes."""
    return json.dumps(doc, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def validate_document(doc):
    if doc.get("bomFormat") != "CycloneDX":
        raise SbomError(f"SBOM invalide : bomFormat={doc.get('bomFormat')!r}")
    if not doc.get("specVersion"):
        raise SbomError("SBOM invalide : specVersion manquant")
    if not isinstance(doc.get("components"), list) or not doc["components"]:
        raise SbomError("SBOM invalide : aucune dépendance listée")


def assert_rfc4122_v5(serial_number):
    """Valide un serialNumber : URN, UUID reparsable, v5, variant RFC 4122."""
    if not isinstance(serial_number, str) or not serial_number.startswith(URN_PREFIX):
        raise SbomError(f"serialNumber sans préfixe {URN_PREFIX} : {serial_number!r}")
    try:
        parsed = uuid.UUID(serial_number[len(URN_PREFIX) :])
    except ValueError as exc:
        raise SbomError(f"serialNumber non parsable : {serial_number!r}") from exc
    if parsed.version != 5:
        raise SbomError(f"serialNumber : version={parsed.version}, attendu 5")
    if parsed.variant != uuid.RFC_4122:
        raise SbomError(f"serialNumber : variant={parsed.variant!r}, attendu RFC 4122")
    if f"{URN_PREFIX}{parsed}" != serial_number:
        raise SbomError(f"serialNumber non canonique : {serial_number!r}")
    return parsed


def canonicalize(doc):
    """Neutralise les champs volatils puis réattribue une identité déterministe."""
    validate_document(doc)
    doc.pop("serialNumber", None)
    metadata = doc.get("metadata")
    if isinstance(metadata, dict):
        metadata.pop("timestamp", None)
    serial = uuid.uuid5(PROJECT_NAMESPACE, canonical_json(doc))
    doc["serialNumber"] = f"{URN_PREFIX}{serial}"
    assert_rfc4122_v5(doc["serialNumber"])
    return doc


def main(argv):
    if len(argv) != 2:
        raise SbomError("usage : sbom-canonicalize.py <sbom.json>")
    path = argv[1]
    with open(path, encoding="utf-8") as fh:
        doc = json.load(fh)

    doc = canonicalize(doc)
    parsed = assert_rfc4122_v5(doc["serialNumber"])

    with open(path, "w", encoding="utf-8") as fh:
        fh.write(canonical_json(doc))
        fh.write("\n")

    print(
        f"SBOM valide + canonique : bomFormat={doc['bomFormat']} "
        f"specVersion={doc['specVersion']} components={len(doc['components'])} "
        f"serialNumber={doc['serialNumber']} "
        f"uuid_version={parsed.version} uuid_variant={parsed.variant}"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except SbomError as error:
        print(f"Erreur : {error}", file=sys.stderr)
        sys.exit(1)

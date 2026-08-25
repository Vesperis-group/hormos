# Politique de sécurité

Hormos est un projet **security-first**. Sa raison d'être touche à des privilèges
sensibles (socket Docker ≈ root sur l'hôte), donc la sécurité prime sur la
vitesse de livraison.

## Versions supportées

Le projet est en *early development* (`0.0.0`). Aucune version n'est encore
supportée en production. Les correctifs de sécurité sont appliqués sur `main`.

## Signaler une vulnérabilité

**Ne créez pas d'issue publique** pour une vulnérabilité.

Utilisez la fonctionnalité **GitHub Security Advisories** du dépôt :
*Security → Report a vulnerability* (Private Vulnerability Reporting).

Merci d'inclure :

- une description du problème et de son impact ;
- les étapes de reproduction ;
- la version / le commit concerné ;
- toute atténuation connue.

Nous accusons réception dans les meilleurs délais et vous tenons informé du
traitement et de la publication éventuelle d'un correctif.

## Principes de sécurité du projet

- Socket Docker traité comme un privilège **quasi-root** — jamais exposé
  tel quel, jamais proxifié brut.
- Bind réseau par défaut sur `127.0.0.1`.
- Aucune interpolation shell (`sh -c`), aucune commande construite par
  concaténation non contrôlée.
- Chaîne d'approvisionnement durcie : dépendances épinglées, actions GitHub
  épinglées par SHA, audit automatisé (cargo-audit / deny / machete, gitleaks),
  SBOM et artefacts signés (cosign keyless + provenance SLSA).

Voir [`docs/security-model.md`](docs/security-model.md) et
[`docs/threat-model.md`](docs/threat-model.md).

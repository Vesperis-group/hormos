# Threat model

Analyse initiale des menaces d'Hormos. Elle reste en partie **prospective** :
plusieurs surfaces décrites n'existent pas encore dans le code, mais elles
guident la conception. Elle est affinée à mesure que les fonctionnalités
arrivent.

Méthode : inspiration STRIDE, appliquée aux surfaces propres à un control plane
conteneurs.

## Actifs à protéger

- L'accès au démon Docker (≈ root sur l'hôte).
- L'intégrité de l'hôte et des conteneurs gérés.
- Les secrets (identifiants de registry, variables d'environnement sensibles).
- L'intégrité de la chaîne de build/release (artefacts distribués).

## Surfaces & menaces

| # | Surface | Menace | Atténuation (conception) |
|---|---------|--------|--------------------------|
| 1 | Socket Docker | Élévation vers root, évasion de conteneur | Jamais exposé/proxifié brut ; API locale explicite ; opérations restreintes |
| 2 | Exposition réseau (API/Web) | Accès non autorisé | Bind `127.0.0.1` par défaut ; authn/authz requises avant toute exposition |
| 3 | `exec` dans un conteneur | Exécution de commande arbitraire | Arguments en tableau, pas de `sh -c` ; contrôle d'accès |
| 4 | Injection de commande | Injection via entrées utilisateur | Aucune interpolation shell ; API `docker`/`compose` en argv |
| 5 | Labels / env | Fuite de secrets | Variables d'environnement jamais collectées ; réédaction des valeurs sensibles là où Hormos les manipule lui-même |
| 6 | Secrets registry / env | Vol d'identifiants | Pas de log en clair ; stockage minimal ; jamais committés |
| 7 | Fichier Compose malveillant | Montages/volumes/priv dangereux | Compose = driver isolé ; validation ; avertissements sur options à risque |
| 8 | Path traversal / archive | Écriture hors périmètre (copy, extract) | Normalisation des chemins ; refus des composants `..` et chemins absolus |
| 9 | Streams (logs/events) | Déni de service (flux illimités) | Canaux **bornés** et contre-pression réelle ; tampons bornés en lignes **et** en octets ; aucune accumulation dans le flux lui-même |
| 10 | CSRF / XSS (Web futur) | Actions forgées, injection | En-têtes stricts, tokens anti-CSRF, échappement ; CSP |
| 11 | Docker distant | MITM, hôte compromis | TLS mutuel exigé ; pas de confiance implicite. **Aujourd'hui : transports distants non compilés et `DOCKER_HOST` distant refusé** |
| 12 | Référence de conteneur | Détournement du chemin d'URL de l'API Docker (`/containers/{ref}/…`) | Jeu de caractères restreint aux noms Docker officiels, longueur bornée, validation **avant** connexion |
| 13 | Sortie terminal | Séquences ANSI injectées via noms de conteneurs/images ou messages moteur | Contrôles C0/`DEL`/C1 remplacés par `U+FFFD` au rendu ; messages moteur tronqués |
| 14 | `inspect` | Divulgation de secrets par les variables d'environnement | Le type de domaine n'a aucun champ d'environnement ; l'adaptateur ne les lit pas |
| 15 | Moteur lent ou bloqué | Déni de service par blocage indéfini de la CLI | Délai maximal fixe sur **chaque** opération ; délai client supérieur, pour un message clair plutôt qu'un abandon opaque |
| 16 | Interface terminal | Sollicitation continue du socket Docker par une session laissée ouverte | Aucun sondage périodique : appel uniquement sur action explicite ou après une action de cycle de vie |
| 17 | Interface terminal | Interception de touches par une session lancée hors terminal interactif | Contrôle du TTY **avant** toute connexion ; refus avec le code `2` |
| 18 | Journal d'un conteneur | Séquences ANSI injectées par le processus surveillé (le contenu du journal est arbitraire) | Décodage incrémental et remplacement de tout contrôle hors `\n` et `\t` par `U+FFFD` ; vers un tube ou un fichier, octets recopiés à l'identique — il n'y a pas de terminal à protéger |
| 19 | Journal d'un conteneur | Épuisement mémoire par une ligne sans fin ou un débit continu | Ligne tronquée à 16 Kio avec marqueur explicite ; report UTF-8 borné à 3 octets ; tampons bornés en lignes et en octets, évictions annoncées |
| 20 | Journal d'un conteneur | UTF-8 invalide provoquant une panique ou une coupure de point de code | Décodage tolérant produisant `U+FFFD` ; troncatures faites sur les **caractères**, jamais sur les octets |
| 21 | Flux d'événements | Rafale d'événements privant l'interface de ses touches ou de son rendu | Sélection **biaisée** contrôle → rendu → flux ; rendu coalescé à 16 ms ; annulation prioritaire sur le flux côté CLI |
| 22 | Flux abandonné | Tâche zombie continuant de lire un socket, ou message affiché dans le mauvais panneau | Un seul flux actif, interrompu par `abort()` — qui débloque aussi un envoi en contre-pression ; génération portée par chaque message et vérifiée à l'arrivée |
| 23 | Événements du moteur | Fuite d'informations par les étiquettes arbitraires d'un conteneur | Seul l'attribut `name` est retenu par l'adaptateur ; aucune autre étiquette n'entre dans le domaine |
| 24 | Journal d'un conteneur | Secret écrit par l'application dans son propre journal | **Non atténué, et assumé** : Hormos ne peut pas distinguer un secret d'une donnée ordinaire, et une réédaction approximative donnerait une fausse assurance. `hormos logs` n'est pas plus sûr que `docker logs` sur ce point |

## Chaîne d'approvisionnement

| Menace | Atténuation |
|--------|-------------|
| Dépendance vulnérable | `cargo audit` (RustSec) en CI |
| Licence non conforme / source inconnue | `cargo deny` (licences, sources, avis) |
| Dépendance morte / surface inutile | `cargo machete` |
| Secret committé | `gitleaks` (CI + PR) |
| Action GitHub compromise (tag mobile) | Épinglage par SHA de commit complet |
| Artefact de release falsifié | SBOM CycloneDX + cosign keyless + provenance SLSA |

## Hors périmètre (à ce stade)

- Fuzzing : reporté jusqu'à l'apparition d'une **vraie surface de parsing/entrée**
  (parseurs Compose, entrées réseau). Voir [development.md](development.md).
- Création/suppression de conteneurs, `exec`, stats, images, volumes,
  réseaux : non implémentés.
- Compose, API, Web : non implémentés (voir [ADR](adr/)).

## Voir aussi

- [security-model.md](security-model.md)
- [streams.md](streams.md)
- [architecture.md](architecture.md)

# Modèle de sécurité

Hormos manipule un privilège critique — l'accès au démon Docker — et adopte donc
une posture **security-first**. Ce document décrit les invariants de sécurité de
conception. Le [threat model](threat-model.md) détaille les menaces associées.

## Le socket Docker est un privilège quasi-root

L'accès à `/var/run/docker.sock` permet de lancer des conteneurs privilégiés, de
monter le système de fichiers hôte et donc d'obtenir un contrôle **équivalent à
root** sur la machine. En conséquence :

- Hormos **n'expose jamais** le socket Docker sur le réseau.
- Hormos **n'agit jamais** comme un proxy Docker brut : chaque opération passe
  par une API locale explicite, avec une surface volontairement réduite.
- Toute future exposition réseau exigera une authentification et une autorisation
  explicites.

## Socket local uniquement, par construction

Le crate `hormos-docker` compile Bollard avec `default-features = false` et la
seule fonctionnalité `pipe` : **le code des transports TCP/HTTP/TLS/SSH n'est pas
dans le binaire**. Il ne s'agit donc pas d'une simple garde à l'exécution, mais
d'une absence de capacité.

En complément, la résolution du point de terminaison refuse explicitement tout
`DOCKER_HOST` en `tcp://`, `http://`, `https://`, `ssh://` ou `npipe://` avec le
code de sortie `8`. La découverte par défaut de Bollard n'est volontairement pas
utilisée : elle ignore silencieusement un `DOCKER_HOST` distant et retombe sur le
socket standard, ce qui masquerait précisément l'erreur que l'on veut signaler.

Aucune option de ligne de commande ne permet de désigner un moteur distant.

## Configuration explicite : échec fermé

`DOCKER_HOST` et `XDG_RUNTIME_DIR` sont lus avec `std::env::var_os` et convertis
explicitement. Une variable **définie mais illisible** (octets non UTF-8) est une
erreur `InvalidInput`, jamais une absence : la traiter comme absente ferait
retomber Hormos sur `/var/run/docker.sock`, c'est-à-dire sur un démon que
l'opérateur n'a pas choisi — un échec ouvert. La valeur brute n'est jamais
réaffichée. Bollard n'acceptant qu'un chemin de socket UTF-8, ce refus ne prive
d'aucune configuration réellement utilisable.

## Validation des références de conteneur

Une référence de conteneur est interpolée par le client Docker dans un chemin
d'URL (`/containers/{ref}/json`). Une valeur contenant `/`, `?`, `#`, `%` ou un
caractère de contrôle permettrait donc d'atteindre un autre point d'API que celui
demandé. Hormos n'accepte que le jeu de caractères des noms Docker officiels
(`[A-Za-z0-9][A-Za-z0-9_.-]*`, 128 caractères au plus), et **valide avant toute
connexion** : une entrée invalide n'ouvre même pas le socket.

## Sorties non fiables

Noms de conteneurs, noms d'images et statuts sont choisis par celui qui a créé le
conteneur. Affichés tels quels, ils peuvent contenir des séquences d'échappement
ANSI capables de réécrire l'affichage d'un terminal ou de masquer des lignes.
Toute chaîne provenant du moteur est donc assainie au rendu : les contrôles C0,
`DEL` et C1 sont remplacés par `U+FFFD`.

Les messages d'erreur renvoyés par le moteur subissent le même traitement et sont
tronqués, afin qu'une erreur ne devienne pas un canal d'affichage arbitraire.

Le TUI, qui écrit en plein écran et positionne lui-même le curseur, applique la
**même** fonction d'assainissement à chaque cellule qu'il dessine ; un test fige
qu'aucune chaîne venue du moteur ne peut piloter l'émulateur de terminal.

## Le contenu d'un journal est arbitraire

Un journal de conteneur est écrit par un processus quelconque : c'est l'entrée la
moins fiable qu'Hormos manipule. Elle est traitée séparément, car une politique
qui supprimerait *tout* contrôle rendrait le journal illisible.

Les octets sont décodés de façon incrémentale : `\n` et `\t` sont conservés,
`\r\n` devient `\n`, tout autre contrôle et tout UTF-8 invalide deviennent
`U+FFFD`. Une ligne est tronquée à 16 Kio avec un marqueur explicite, et le
report d'une séquence UTF-8 incomplète est borné à trois octets : ni une ligne
sans fin, ni un flux d'octets invalides ne peuvent épuiser la mémoire.

L'assainissement est décidé **par sortie**, pas globalement : vers un terminal,
les octets sont décodés ; vers un tube ou un fichier, ils sont recopiés à
l'identique. Il n'y a pas de terminal à protéger au bout d'un tube, et altérer
les octets casserait toute chaîne de traitement. Détail :
[streams.md](streams.md).

**Hormos ne réédite pas les secrets qu'une application écrit dans son propre
journal** : il ne peut pas les distinguer d'une donnée ordinaire, et une
réédaction approximative donnerait une fausse assurance. Sur ce point,
`hormos logs` n'est ni plus ni moins sûr que `docker logs`.

## Un flux ne peut pas affamer l'interface

Un conteneur bavard est un déni de service potentiel. Trois garanties le
neutralisent : les canaux internes sont **bornés**, ce qui remonte une vraie
contre-pression jusqu'au moteur au lieu de laisser une file grossir ; la
sélection est **biaisée** dans l'ordre contrôle, rendu, flux, de sorte qu'une
rafale ne peut ni retarder une touche ni empêcher le redessin ; et côté CLI,
l'annulation est examinée avant le flux, pour qu'un `Ctrl+C` reste immédiat.

Un seul flux est actif à la fois. L'ouverture d'un autre interrompt le précédent
par `abort()`, ce qui referme la requête HTTP et débloque une tâche arrêtée sur
un envoi ; l'interruption n'étant pas instantanée, chaque message porte une
génération vérifiée à l'arrivée, afin qu'un flux abandonné n'écrive jamais dans
le panneau d'un autre.

## L'interface terminal n'interroge le moteur que sur demande

Le TUI ne sonde **jamais** le moteur en arrière-plan : il n'émet un appel que
pour une action explicite de l'utilisateur, ou juste après une action de cycle de
vie. Une session ouverte et inactive, sans flux ouvert, ne produit aucun trafic
sur le socket Docker. Un flux est ouvert à la demande, un seul à la fois, et une
interruption n'est **jamais** suivie d'une reconnexion automatique : rien ne
martèle un moteur en panne. Hors d'un terminal interactif, `hormos` et `hormos tui` refusent de
démarrer **avant** d'ouvrir le socket. Voir [tui.md](tui.md).

## Les variables d'environnement ne sont jamais lues

`docker inspect` expose la configuration complète d'un conteneur, y compris ses
variables d'environnement — qui contiennent régulièrement des secrets. Le type de
domaine `ContainerDetails` **n'a aucun champ** pour les accueillir et
l'adaptateur ne les lit pas. La sortie `--json` ne peut donc pas les divulguer.

## Bind local par défaut

Les futures interfaces réseau (API, Web) écoutent par défaut sur `127.0.0.1`.
L'écoute sur une interface publique doit être un choix explicite et documenté de
l'utilisateur, jamais un défaut.

## Pas d'interpolation shell

Les commandes externes (`docker`, `docker compose`) sont invoquées avec un
**tableau d'arguments explicite**. Hormos n'utilise jamais `sh -c`, ni de
concaténation de chaînes pour construire une commande. Cela élimine par
conception l'injection de commande.

## Les tests ne détruisent que ce qu'ils ont créé

La suite d'intégration parle à un démon **réel**, potentiellement partagé. Toute
sélection destinée à une suppression croise l'étiquette générique
`io.hormos.test=true` **et** l'identité de l'exécution
(`io.hormos.test.run=<HORMOS_DOCKER_TEST_RUN_ID>`) ; le nettoyage d'une fixture
cible en outre son identité individuelle. Filtrer sur la seule étiquette
générique suffirait à supprimer les conteneurs d'une autre suite Hormos : c'est
interdit, au même titre que `docker prune`. Un test dédié vérifie que la
sélection d'une exécution ne franchit jamais la frontière d'une autre.

## Chaîne d'approvisionnement (supply chain)

- Toolchain Rust **épinglée** (`rust-toolchain.toml`).
- Dépendances **verrouillées** (`Cargo.lock` committé, builds `--locked`).
- `unsafe` **interdit** au niveau workspace (`unsafe_code = "forbid"`).
- Audit automatisé : `cargo audit` (RustSec), `cargo deny` (licences, sources,
  avis), `cargo machete` (dépendances mortes), `gitleaks` (secrets).
- Actions GitHub **épinglées par SHA** de commit complet.
- Release : SBOM CycloneDX, signature **cosign keyless** (OIDC, aucun secret long
  terme) et **provenance SLSA**.

## Gestion des données sensibles

- Aucun secret n'est committé (gitleaks en CI + revue).
- Les variables d'environnement des conteneurs ne sont **jamais** collectées ni
  affichées (voir plus haut).
- Les logs et sorties destinés à l'utilisateur évitent de divulguer des
  identifiants ; la réédaction sera systématique là où des secrets peuvent
  transiter (env, labels, URLs).

## Moindre privilège partout

- Permissions GitHub Actions minimales (`contents: read` par défaut).
- GitHub App de release aux permissions minimales (Contents RW, Pull Requests RO,
  Metadata RO), installée uniquement sur `hormos` (voir [release-app.md](release-app.md)).

## Voir aussi

- [threat-model.md](threat-model.md)
- [streams.md](streams.md)
- [architecture.md](architecture.md)
- [../SECURITY.md](../SECURITY.md)

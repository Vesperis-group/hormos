# Flux temps réel (journaux et événements)

Deux flux existent aujourd'hui : le **journal** d'un conteneur
(`hormos logs`, touche `l` du TUI) et les **événements** du moteur
(`hormos events`, touche `e`). Ils partagent une même abstraction,
`hormos_core::stream::RuntimeStream<T>`, et les mêmes règles de bornage.

## Le contrat

Un `RuntimeStream<T>` est une suite d'éléments du domaine produits au fil de
l'eau. Le type est **opaque** : aucune interface ne voit `futures`, ni Bollard,
ni HTTP. Il se consomme par une seule méthode, `next()`.

Trois propriétés tiennent tout le reste :

- **il n'accumule rien.** Chaque élément est produit, remis à l'appelant, puis
  relâché. Un conteneur qui écrit pendant trois jours ne fait pas grossir le
  flux d'un octet. La rétention est la responsabilité de ce qui affiche ;
- **une erreur en cours de flux est un élément comme un autre.** Elle est
  transmise, pas propagée par `?` : ce qui a déjà été reçu reste acquis ;
- **le détruire l'annule.** Il n'y a ni jeton d'annulation, ni message d'arrêt :
  abandonner la valeur referme la requête HTTP sous-jacente.

## Ouverture paresseuse

Ouvrir un flux ne parle pas encore au moteur. La requête HTTP n'est émise qu'à
la **première lecture**.

C'est sans conséquence pour un journal, qui rejoue son historique. Ça en a une
pour les événements, qui ne se rejouent pas : s'abonner puis agir sur le moteur
sans avoir lu une seule fois manque les événements de cette action. Il faut donc
**commencer à lire avant** de provoquer ce que l'on veut observer. Un test
d'intégration (`the_engine_reports_the_lifecycle_of_a_container`) fige la
contrainte ; il échouait avant qu'elle ne soit comprise.

## Délais

Les opérations ponctuelles sont bornées par un délai maximal fixe. Les flux ne
le sont **pas**, et c'est voulu : un `hormos logs -f` doit pouvoir vivre des
heures. Le `client_timeout` de Bollard ne borne d'ailleurs que l'obtention des
en-têtes HTTP, pas la lecture du corps — un flux suivi n'est donc jamais
interrompu par lui. Le bornage des flux est ailleurs : dans la mémoire, décrite
plus bas, et dans la capacité de l'utilisateur à interrompre (`Ctrl+C`, `Échap`).

## Traitement des octets d'un journal

Le moteur livre des octets bruts, potentiellement hostiles : ils viennent d'un
processus quelconque. Deux primitives partagées par la CLI et le TUI les
traitent, dans `hormos_core::logs`.

**`LogDecoder`** décode l'UTF-8 de façon incrémentale et neutralise ce qui peut
piloter un terminal :

| Entrée                                   | Sortie     |
| ---------------------------------------- | ---------- |
| `\n`, `\t`                               | conservés  |
| `\r\n`                                   | `\n`       |
| `\r` isolé                               | `U+FFFD`   |
| autre contrôle C0, `DEL`, contrôle C1    | `U+FFFD`   |
| séquence UTF-8 invalide                   | `U+FFFD`   |
| séquence UTF-8 coupée en fin de fragment | reportée   |

La politique diffère volontairement de `display::sanitize`, qui remplace *tout*
contrôle : un journal sans retour à la ligne ne serait plus un journal.

Un `\r` en fin de fragment est mis en attente : un `\r\n` coupé entre deux
fragments doit rester un simple retour à la ligne. Le report d'une séquence UTF-8
incomplète est borné à trois octets — un journal qui n'émettrait jamais de
caractère valide ne peut pas faire croître la mémoire.

**Un décodeur par sortie.** `stdout` et `stderr` arrivent entrelacés ; recoller
leurs fragments produirait des lignes mélangées et de l'UTF-8 invalide à chaque
alternance.

**`LogFramer`** ajoute le découpage en lignes et une borne de longueur : au-delà
de 16 Kio, le reste de la ligne est abandonné et la ligne porte
`…[ligne tronquée]`. Une ligne infinie ne peut donc pas épuiser la mémoire. Le
découpeur n'accumule jamais de ligne complète en interne.

## Ce que fait la CLI

L'assainissement est décidé **par sortie**, pas globalement :

- vers un **terminal**, les octets passent par le décodeur ;
- vers un **fichier ou un tube**, ils sont recopiés à l'identique.

Il n'y a pas de terminal à protéger au bout d'un tube, et altérer les octets
casserait toute chaîne de traitement (`hormos logs web | grep`). `hormos logs web
> fichier` assainit donc encore `stderr` s'il est resté attaché au terminal.

Interrompre un suivi est un **succès** (code `0`). La sélection entre le flux et
l'annulation est `biased`, l'annulation d'abord : un conteneur qui inonde sa
sortie ne peut pas retarder un `Ctrl+C`. Un tube fermé en aval (`| head`) est
traité comme une fin normale, pas comme une erreur.

`hormos events --json` produit du **NDJSON** : un objet complet par ligne, vidé
immédiatement. Un flux suivi ne refermerait jamais un tableau JSON.

## Ce que fait le TUI

### Deux canaux, une boucle

Les touches et les résultats d'appels ponctuels arrivent par le canal de
*contrôle* ; les flux par un canal séparé. La sélection est **biaisée** dans cet
ordre : **contrôle, rendu, flux**.

L'ordre est le cœur du dispositif. Un conteneur qui écrit sans discontinuer ne
peut ni retarder une touche, ni empêcher l'écran de se redessiner : les deux
branches qui comptent sont examinées avant la sienne.

Les deux canaux sont **bornés**. Un flux plus rapide que l'affichage attend sur
son envoi ; cette attente remonte la contre-pression jusqu'au moteur, au lieu de
laisser une file grossir sans limite. Aucun canal non borné, aucun abandon
silencieux d'élément.

### Rendu coalescé

Le rendu n'est pas déclenché par chaque message mais par une horloge de 16 ms :
sous une rafale, l'écran est dessiné une fois par image plutôt qu'une fois par
ligne reçue.

### Un seul flux à la fois

Le TUI n'affiche qu'un panneau ; deux abonnements simultanés doubleraient la
charge sans rien montrer de plus. Ouvrir un flux interrompt le précédent :
`abort()` referme la requête HTTP **et** débloque une tâche arrêtée sur un envoi
en contre-pression — sans quoi elle ne se terminerait jamais.

L'interruption n'étant pas instantanée, chaque flux porte une **génération**. Un
message issu d'un flux abandonné est écarté à l'arrivée, plutôt qu'affiché dans
le mauvais panneau.

### Bornes de rétention

| Élément                    | Borne   |
| -------------------------- | ------- |
| Lignes de journal          | 2 000   |
| Octets de journal          | 2 Mio   |
| Longueur d'une ligne       | 16 Kio  |
| Événements                 | 500     |
| Historique à l'ouverture   | 200 lignes |

Deux bornes plutôt qu'une pour le journal : 2 000 lignes de 16 Kio pèseraient
32 Mio. La plus stricte l'emporte. Ce que l'éviction a emporté est **annoncé**
en en-tête, jamais passé sous silence.

### Défilement

L'ancre est comptée **depuis le début** du tampon et corrigée à chaque éviction.
Une ancre comptée depuis la fin donnerait l'illusion de la stabilité tout en
glissant d'une ligne à chaque arrivée. Revenir au bas rétablit le suivi
automatique ; tant qu'on ne l'a pas fait, l'en-tête indique « en pause ».

### Erreurs et reconnexion

Une panne en cours de flux **conserve** ce qui a été reçu et affiche sa cause :
dix minutes de journal ne disparaissent pas parce que le moteur a coupé. La
reconnexion est **manuelle** (`R`) : un flux qui se rétablirait seul martèlerait
un moteur en panne et rejouerait le début du journal à chaque tentative. Elle
repart d'un tampon vide, la nouvelle connexion renvoyant son propre historique.

## Ce qui n'est volontairement pas fait

- **Aucun filtre** sur le flux d'événements. Un filtre mal formé donne un flux
  silencieusement vide, ce qui est pire qu'un flux bavard.
- **Aucune option d'événement** exposée : aucune n'est utile aujourd'hui, et
  Hormos n'anticipe pas.
- **Aucun rafraîchissement automatique** de la liste de conteneurs sur
  événement : il demanderait deux flux simultanés pour un gain que `R` couvre.
- **Aucune réédaction de secret** dans les journaux : Hormos ne peut pas
  distinguer un mot de passe d'un identifiant, et une réédaction approximative
  donnerait une fausse assurance. Voir [threat-model.md](threat-model.md).

## Voir aussi

- [tui.md](tui.md)
- [architecture.md](architecture.md)
- [security-model.md](security-model.md)
- [threat-model.md](threat-model.md)

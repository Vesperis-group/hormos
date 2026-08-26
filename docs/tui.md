# Interface terminal (TUI)

`hormos` sans sous-commande — ou `hormos tui` — ouvre une interface plein écran
sur les conteneurs du moteur **Docker local**. C'est une seconde peau sur le même
cœur : `hormos-tui` ne dépend ni de Bollard ni de `hormos-docker`, il ne connaît
que `ContainerService` et le domaine d'`hormos-core`. « One engine. Every
interface. »

## Portée

Le TUI expose exactement ce que la CLI sait déjà faire, rien de plus :

- lister les conteneurs (en cours, ou tous) ;
- inspecter un conteneur ;
- `start`, `stop`, `restart` ;
- suivre le **journal** d'un conteneur et les **événements** du moteur.

Il ne crée pas, ne supprime pas, n'exécute aucune commande dans un conteneur.

## Touches

| Touche          | Effet                                                      |
| --------------- | ---------------------------------------------------------- |
| `q`             | Quitter                                                    |
| `↑` `↓` `k` `j` | Naviguer dans la liste                                     |
| `a`             | Alterner « en cours » / « tous » (déclenche un rafraîchissement) |
| `R`             | Rafraîchir la liste                                        |
| `/`             | Filtrer ; `Échap` annule, `Entrée` valide                  |
| `i`             | Détail du conteneur sélectionné                            |
| `l`             | Journal du conteneur sélectionné                           |
| `e`             | Événements du moteur                                       |
| `s`             | `start` si arrêté, `stop` si en cours                      |
| `r`             | `restart`                                                  |
| `?`             | Aide                                                       |
| `Échap`         | Ferme le panneau ouvert ; sur la liste, quitte             |

Devant un journal ou des événements, les touches changent de sens : `↑` `↓`
`k` `j` défilent, `PgPréc` / `PgSuiv` page par page, `Début` / `Fin` vont aux
extrémités — `Fin` rétablit le suivi automatique —, `R` reconnecte et `Échap` ou
`q` reviennent à la liste. Les touches de la liste n'y ont aucun effet.

La touche `s` est **contextuelle** : elle applique le verbe qui a un sens pour
l'état courant, et rien d'autre. Une action déjà en vol sur un conteneur bloque
toute nouvelle action sur ce même conteneur jusqu'à son résultat.

## Ce que le TUI ne fait pas

- **Aucun sondage périodique.** Hormos n'interroge le moteur que sur une action
  explicite (`a`, `R`, `i`, `l`, `e`, une action de cycle de vie) ou juste après
  une action. Un TUI ouvert et inactif, sans flux ouvert, ne produit **aucun
  trafic** sur le socket Docker : c'est un choix de sécurité autant que de
  sobriété. Un flux, lui, est ouvert **à la demande** et un seul à la fois.
- **Aucune reconnexion automatique.** Un flux interrompu le reste jusqu'à ce que
  l'utilisateur appuie sur `R` : rien ne martèle un moteur en panne.
- **Aucune modification destructive.** Pas de suppression, pas de `prune`, pas
  d'exécution de commande dans un conteneur.
- **Aucun accès distant.** Le TUI hérite de la résolution de point de
  terminaison de la CLI : socket Unix local uniquement.

## Robustesse d'affichage

Toute chaîne provenant du moteur — nom, image, état, message d'erreur — passe par
`hormos_core::display::sanitize` avant d'atteindre le terminal. Un nom de
conteneur contenant des séquences d'échappement ANSI ne peut donc **pas** piloter
l'émulateur de terminal de l'utilisateur. Un test dédié
(`hostile_strings_cannot_drive_the_terminal`) fige ce comportement.

En dessous de **60×12**, l'interface affiche un écran demandant plus de place
plutôt que de dessiner un tableau illisible.

## Comportement en cas d'erreur

- Un échec de rafraîchissement **conserve** la liste précédente et affiche le
  message en pied d'écran : une erreur passagère ne vide pas l'écran.
- L'écran d'erreur plein n'apparaît que si aucune liste n'a jamais pu être
  affichée (moteur injoignable au démarrage, par exemple) ; il propose de
  réessayer avec `R`.
- Les erreurs du moteur **ne terminent jamais** le TUI. Seule une défaillance du
  terminal lui-même le fait, avec le code de sortie `1`.

## Restitution du terminal

- **Sortie normale ou erreur** : le `Drop` de `TerminalGuard` rend le curseur,
  quitte l'écran alternatif et désactive le mode brut. Le terminal est
  intégralement restauré.
- **Panique** : le *panic hook* installé par `ratatui` restaure l'écran avant
  d'afficher la panique.

> **Limite connue et assumée.** Le profil `release` du workspace utilise
> `panic = "abort"` : sur une panique, les `Drop` ne s'exécutent pas. Le *panic
> hook* restaure alors le mode brut et l'écran alternatif, mais **pas le
> curseur**, qui peut rester masqué. Un `reset` suffit à revenir à la normale. Le
> profil `dev` n'est pas concerné. La corriger demanderait d'abandonner
> `panic = "abort"` ou d'installer un gestionnaire de panique propre à Hormos :
> deux prix trop élevés pour un curseur.

## Arrêt du fil de lecture

Le terminal ne doit jamais être restauré pendant qu'un fil lit encore l'entrée
standard. La destruction du lecteur demande donc l'arrêt, réveille le fil, puis
l'attend.

L'envoi vers la boucle principale est **annulable** : sur un canal saturé, il
réessaie par courtes tranches et vérifie l'arrêt entre deux tentatives, au lieu
de s'endormir jusqu'à ce que quelqu'un vide le canal. Sans cela, une rafale de
touches suivie d'un `q` pourrait figer la sortie : le fil attendrait un
consommateur déjà terminé. L'arrêt ne dépend ainsi **jamais** du fait que la
boucle principale continue à consommer.

Délai maximal d'arrêt : **200 ms**, le temps que la lecture du terminal — seul
point d'attente qu'un réveil ne peut pas interrompre — rende la main.

## Sortie non interactive

Si la sortie standard n'est pas un terminal (redirection, tube, CI), `hormos` et
`hormos tui` refusent de démarrer avec le code `2`, **avant** toute connexion au
moteur, et renvoient vers `hormos ps`. Rediriger `hormos` dans un fichier ne
produit donc ni écran de contrôle, ni ouverture de socket.

## Architecture interne

```
   ┌──────────────┐  contrôle (64)  ┌─────────┐   Command   ┌──────────────────┐
   │ fil clavier  │────────────────▶│   App   │────────────▶│ ContainerService │
   │ (crossterm)  │                 │  (pur)  │  tâche      │   (hormos-core)  │
   └──────────────┘                 └────┬────┘  tokio      └────────┬─────────┘
   ┌──────────────┐   flux (256)         │ rendu (16 ms)             │
   │ tâche de flux│──────────────────────┤                           │ Message
   └──────────────┘                      ▼                           │
          ▲                         ┌─────────┐                      │
          └── RuntimeStream ────────│   ui    │◀─────────────────────┘
                                    └─────────┘
```

- `app` est **pur** : ni terminal, ni Docker, ni horloge. Ses transitions sont
  testées intégralement (`Message` en entrée, `Option<Command>` en sortie).
- La lecture du clavier vit sur un **fil système dédié** qui publie dans un canal
  `tokio` **borné**, et non sur l'`event-stream` de crossterm : si l'interface
  prend du retard, la lecture ralentit au lieu d'accumuler des touches, et
  l'envoi reste annulable, pour que la contre-pression ne puisse jamais empêcher
  l'arrêt. Un flux asynchrone de touches n'offrirait ni l'un ni l'autre.
- Les commandes partent dans leur propre tâche : un `stop` qui consomme son délai
  de grâce ne fige pas l'affichage.
- La sélection est mémorisée **par identifiant**, pas par ligne : un
  rafraîchissement qui réordonne la liste ne déplace pas le curseur sous les
  doigts de l'utilisateur.
- Les flux arrivent par un **second canal borné**, et la sélection est biaisée
  dans l'ordre contrôle, rendu, flux : un conteneur qui inonde sa sortie ne peut
  ni retarder une touche, ni empêcher l'écran de se redessiner. Le détail des
  bornes, des générations et du défilement est dans [streams.md](streams.md).

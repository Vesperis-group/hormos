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
- `start`, `stop`, `restart`.

Il ne crée pas, ne supprime pas, n'attache aucun flux, ne lit aucun journal.

## Touches

| Touche          | Effet                                                      |
| --------------- | ---------------------------------------------------------- |
| `q`             | Quitter                                                    |
| `↑` `↓` `k` `j` | Naviguer dans la liste                                     |
| `a`             | Alterner « en cours » / « tous » (déclenche un rafraîchissement) |
| `R`             | Rafraîchir la liste                                        |
| `/`             | Filtrer ; `Échap` annule, `Entrée` valide                  |
| `i`             | Détail du conteneur sélectionné                            |
| `s`             | `start` si arrêté, `stop` si en cours                      |
| `r`             | `restart`                                                  |
| `?`             | Aide                                                       |
| `Échap`         | Ferme le panneau ouvert ; sur la liste, quitte             |

La touche `s` est **contextuelle** : elle applique le verbe qui a un sens pour
l'état courant, et rien d'autre. Une action déjà en vol sur un conteneur bloque
toute nouvelle action sur ce même conteneur jusqu'à son résultat.

## Ce que le TUI ne fait pas

- **Aucun sondage périodique.** Hormos n'interroge le moteur que sur une action
  explicite (`a`, `R`, `i`, une action de cycle de vie) ou juste après une
  action. Un TUI ouvert et inactif ne produit **aucun trafic** sur le socket
  Docker : c'est un choix de sécurité autant que de sobriété.
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

`TerminalGuard` restaure le mode brut, l'écran alternatif et le curseur dans son
`Drop`, et le point d'entrée de `ratatui` installe un *panic hook* qui restaure
l'écran avant d'afficher la panique.

> **Limite connue.** Le profil `release` du workspace utilise `panic = "abort"` :
> sur une panique, les `Drop` ne s'exécutent pas. Le *panic hook* restaure alors
> l'écran, mais **pas le curseur**. Un `reset` suffit à revenir à la normale. Le
> profil `dev` n'est pas concerné.

## Sortie non interactive

Si la sortie standard n'est pas un terminal (redirection, tube, CI), `hormos` et
`hormos tui` refusent de démarrer avec le code `2`, **avant** toute connexion au
moteur, et renvoient vers `hormos ps`. Rediriger `hormos` dans un fichier ne
produit donc ni écran de contrôle, ni ouverture de socket.

## Architecture interne

```
      ┌──────────────┐   Message   ┌─────────┐   Command   ┌──────────────────┐
      │ fil clavier  │────────────▶│   App   │────────────▶│ ContainerService │
      │ (crossterm)  │  canal (64) │  (pur)  │  tâche      │   (hormos-core)  │
      └──────────────┘             └────┬────┘  tokio      └────────┬─────────┘
                                        │ rendu                     │ Message
                                        ▼                           │
                                   ┌─────────┐                      │
                                   │   ui    │◀─────────────────────┘
                                   └─────────┘
```

- `app` est **pur** : ni terminal, ni Docker, ni horloge. Ses transitions sont
  testées intégralement (`Message` en entrée, `Option<Command>` en sortie).
- La lecture du clavier vit sur un **fil système dédié** qui publie dans un canal
  `tokio` **borné**. Pas de `event-stream`, donc pas de `futures-core` ni de
  `signal-hook-mio` supplémentaires ; et si l'interface prend du retard, la
  lecture ralentit au lieu d'accumuler des touches.
- Les commandes partent dans leur propre tâche : un `stop` qui consomme son délai
  de grâce ne fige pas l'affichage.
- La sélection est mémorisée **par identifiant**, pas par ligne : un
  rafraîchissement qui réordonne la liste ne déplace pas le curseur sous les
  doigts de l'utilisateur.

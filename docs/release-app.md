# GitHub App de release (`vesperis-hormos-release`)

Le workflow [`.github/workflows/release.yml`](../.github/workflows/release.yml)
publie chaque release d'`hormos` (commit de release, tag `vX.Y.Z`, GitHub Release
et artefacts signés). Le commit et le tag de release sont créés **via l'API
GitHub** avec le jeton de l'App : le commit est donc signé par GitHub et
**Verified** (pas seulement autorisé par bypass), sans PAT ni clé de signature
dédiée. Les rulesets protègent `main` et `v*` ; l'App est le **seul** acteur
autorisé à les contourner.

> **Pourquoi une App et pas un PAT ?** Le jeton de l'App est **généré à
> l'exécution**, à **durée de vie courte** (≈ 1 h), **limité au dépôt** `hormos`
> et aux **permissions minimales** de l'App. Aucun secret long terme, surface
> réduite, révocation immédiate par désinstallation.

> **Commit Verified via API.** Un `git commit` local dans le runner ne serait pas
> signé (aucune clé) : il ne serait que *bypassé*, pas Verified. En créant le
> commit via la Git Data API avec le jeton de l'App, GitHub le signe et le
> marque `Verified` (auteur `vesperis-hormos-release[bot]`). Le script
> `scripts/create-release-commit.sh` échoue si le commit n'est pas Verified.
> Le jeton n'est utilisé que dans les étapes nécessaires ; le checkout est en
> `persist-credentials: false` (aucun identifiant résiduel), et le `fetch` du tag
> exact utilise un en-tête d'authentification ponctuel non persisté.

## 1. Vue d'ensemble

| Élément | Valeur |
| --- | --- |
| Nom de l'App | `vesperis-hormos-release` |
| Propriétaire | organisation `Vesperis-group` |
| Installation | **uniquement** sur le dépôt `hormos` |
| Action consommatrice | `actions/create-github-app-token` (épinglée par SHA) |
| Variable Actions | `HORMOS_RELEASE_APP_CLIENT_ID` |
| Secret Actions | `HORMOS_RELEASE_APP_PRIVATE_KEY` |
| Garde de publication | variable `HORMOS_RELEASE_ENABLED` = `true` |

> **Statut actuel (observé en lecture seule)** : l'App est créée et installée ;
> la variable `HORMOS_RELEASE_APP_CLIENT_ID` et le secret
> `HORMOS_RELEASE_APP_PRIVATE_KEY` sont configurés. Les rulesets « Protect main »
> et « Protect release tags » existent et incluent déjà l'App (acteur
> *Integration*) dans leurs *bypass lists* ; « Protect main » liste aussi la team
> `hormos-maintainers`. Il reste uniquement à positionner
> `HORMOS_RELEASE_ENABLED=true` après validation de bout en bout. La politique des
> rulesets n'est pas modifiée par cette PR.

Le workflow lit ces entrées :

```yaml
- name: Generate GitHub App token
  id: app-token
  uses: actions/create-github-app-token@<sha> # v3.2.0
  with:
    client-id: ${{ vars.HORMOS_RELEASE_APP_CLIENT_ID }}
    private-key: ${{ secrets.HORMOS_RELEASE_APP_PRIVATE_KEY }}
    owner: ${{ github.repository_owner }}
    repositories: hormos
```

## 2. Permissions de l'App (moindre privilège)

**Repository permissions** — exactement ceci, rien de plus :

| Permission | Niveau | Pourquoi |
| --- | --- | --- |
| Contents | **Read and write** | pousser le commit + le tag, créer la Release |
| Pull requests | **Read-only** | lecture du contexte des PR (changelog) |
| Metadata | Read-only (auto) | requis par GitHub pour toute App |

**À NE PAS accorder** : Administration, Actions (write), Issues, Secrets,
Workflows, Packages, ni aucune permission d'organisation.

**Webhook** : décocher **Active** (aucun webhook nécessaire).

## 3. Création de l'App (rappel — déjà réalisé)

1. Organisation `Vesperis-group` → Settings → Developer settings → GitHub Apps →
   New GitHub App.
2. Name : `vesperis-hormos-release` ; Homepage : URL du dépôt.
3. Webhook : décocher **Active**.
4. Repository permissions : tableau de la section 2.
5. « Where can this GitHub App be installed? » → **Only on this account**.
6. Récupérer le **Client ID** (chaîne `Iv23li…`, différente de l'App ID) →
   variable `HORMOS_RELEASE_APP_CLIENT_ID`.
7. Générer une **clé privée** (`.pem`, téléchargeable une seule fois) → secret
   `HORMOS_RELEASE_APP_PRIVATE_KEY`.
8. Install App → **Only select repositories** → `hormos` uniquement.

## 4. Variable et secret (déjà configurés)

Dans **`hormos` → Settings → Secrets and variables → Actions** :

- Variables → `HORMOS_RELEASE_APP_CLIENT_ID` = Client ID (`Iv23li…`).
- Secrets → `HORMOS_RELEASE_APP_PRIVATE_KEY` = contenu complet du `.pem`.

Le Client ID n'est pas sensible (variable) ; seule la clé privée est un secret.

## 5. Ajouter l'App aux *bypass lists* des rulesets (à faire)

Les rulesets protègent `main` et les tags `v*` (voir
[repository-rules.md](repository-rules.md)). Pour publier, l'App — et **elle
seule** — doit pouvoir les contourner :

- ruleset **« Protect main »** → Bypass list → Add bypass →
  `vesperis-hormos-release`.
- ruleset **« Protect release tags »** → Bypass list → Add bypass →
  `vesperis-hormos-release`.

Aucun utilisateur humain ni le `GITHUB_TOKEN` par défaut ne doit y figurer.

## 6. Activer la publication

La publication est **gardée** : le job `publish` ne s'exécute que si la variable
de dépôt `HORMOS_RELEASE_ENABLED` vaut `true`. Tant qu'elle est absente, un push
sur `main` ne tente aucune publication et n'échoue pas faute de secrets.

Une fois l'App vérifiée de bout en bout et les bypass en place :

- Settings → Secrets and variables → Actions → Variables →
  `HORMOS_RELEASE_ENABLED` = `true`.

## 7. Rotation et révocation

- **Rotation** : générer une nouvelle clé privée, mettre à jour le secret, puis
  supprimer l'ancienne clé côté App.
- **Révocation d'urgence** : désinstaller l'App du dépôt coupe immédiatement toute
  publication ; aucun jeton long terme ne subsiste (token éphémère).

## 8. Rappels de sécurité

- Aucun PAT long terme pour la release.
- Jeton App généré à l'exécution, expiration rapide, masqué dans les logs.
- Clé privée jamais committée ni journalisée.
- Permissions minimales ; App installée **uniquement** sur `hormos`.

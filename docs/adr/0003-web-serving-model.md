# ADR 0003 — Modèle de service Web

- **Statut** : accepté
- **Date** : 2026-08
- **Contexte** : bootstrap Hormos (ni API ni Web implémentés)

## Contexte

Hormos proposera à terme une interface Web. Il faut décider comment elle est
servie et comment elle communique avec le cœur, sans multiplier les composants ni
la surface d'attaque.

## Décision

- Le Web sera **servi par l'API Axum** (fichiers statiques + endpoints), **sans
  conteneur frontend obligatoire**. Un seul binaire sert l'interface et l'API.
- Les échanges suivent une **API locale explicite**, jamais un proxy transparent
  du socket Docker.
- Interactivité (logs en flux, events, exec) via **REST + SSE/WebSocket**,
  uniquement là où l'interactivité l'exige ; le reste reste du REST simple.
- Bind par défaut sur **`127.0.0.1`**. Toute exposition publique est un choix
  explicite, avec authentification/autorisation.
- Menaces Web (CSRF, XSS) traitées dès l'introduction du frontend : en-têtes
  stricts, CSP, tokens anti-CSRF, échappement systématique.

Rien de tout cela n'est implémenté dans ce bootstrap (pas d'Axum, pas de
React/Vite, pas de crate vide).

## Conséquences

- **+** Déploiement simple (un binaire), surface réduite, pas d'orchestration
  frontend.
- **+** Cohérence : le Web consomme le même cœur que la CLI/TUI.
- **−** Le binaire embarque les assets Web (taille) ; acceptable.

## Alternatives écartées

- **Conteneur frontend séparé** (Nginx/Node) : composant et surface
  supplémentaires, complexité de déploiement injustifiée.
- **Proxy direct du socket Docker vers le navigateur** : dangereux (privilège
  quasi-root exposé) — explicitement refusé.

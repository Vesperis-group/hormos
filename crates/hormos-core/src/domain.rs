//! Types du domaine Hormos.
//!
//! Ces structures sont **indépendantes du moteur** : aucun type généré par un
//! client Docker n'y apparaît. Les adaptateurs (`hormos-docker`) traduisent les
//! réponses du moteur vers ces types et ne conservent rien d'autre.
//!
//! Elles restent délibérément **minimales** : ni clone de `docker info`, ni
//! clone de `docker inspect`. En particulier, les **variables d'environnement**
//! des conteneurs ne sont ni collectées ni exposées : elles contiennent
//! régulièrement des secrets.

use serde::Serialize;

/// Informations essentielles sur le moteur de conteneurs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemInfo {
    /// Version du serveur (ex. `29.7.2`).
    pub server_version: Option<String>,
    /// Version de l'API négociée avec le serveur (ex. `1.51`).
    pub api_version: Option<String>,
    /// Système d'exploitation du serveur (ex. `linux`).
    pub os: Option<String>,
    /// Architecture du serveur (ex. `x86_64`).
    pub architecture: Option<String>,
    /// Nombre total de conteneurs connus du moteur.
    pub containers_total: Option<i64>,
    /// Nombre de conteneurs en cours d'exécution.
    pub containers_running: Option<i64>,
    /// Point de terminaison local réellement utilisé (ex. chemin du socket).
    pub endpoint: String,
}

/// État d'un conteneur, normalisé.
///
/// La variante [`ContainerState::Other`] conserve la valeur brute renvoyée par
/// le moteur pour les états que nous ne connaissons pas encore : Hormos ne perd
/// jamais l'information, mais ne fait pas semblant de la comprendre.
///
/// La sérialisation est une **simple chaîne** (`"running"`), et non un objet
/// balisé : la sortie `--json` reste ainsi stable et directement exploitable par
/// `jq` quel que soit l'état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    /// Créé, jamais démarré.
    Created,
    /// En cours de redémarrage.
    Restarting,
    /// En cours d'exécution.
    Running,
    /// En cours de suppression.
    Removing,
    /// Suspendu.
    Paused,
    /// Terminé.
    Exited,
    /// Mort (le moteur n'a pas pu le nettoyer).
    Dead,
    /// État inconnu d'Hormos, conservé tel quel.
    Other(String),
}

impl ContainerState {
    /// Normalise un état textuel renvoyé par le moteur.
    #[must_use]
    pub fn from_engine(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "created" => Self::Created,
            "restarting" => Self::Restarting,
            "running" => Self::Running,
            "removing" => Self::Removing,
            "paused" => Self::Paused,
            "exited" => Self::Exited,
            "dead" => Self::Dead,
            _ => Self::Other(value.to_owned()),
        }
    }

    /// Représentation textuelle stable de l'état.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Created => "created",
            Self::Restarting => "restarting",
            Self::Running => "running",
            Self::Removing => "removing",
            Self::Paused => "paused",
            Self::Exited => "exited",
            Self::Dead => "dead",
            Self::Other(value) => value,
        }
    }

    /// Indique si le conteneur est en cours d'exécution.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

impl Serialize for ContainerState {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Résumé d'un conteneur, tel que listé par `hormos ps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerSummary {
    /// Identifiant complet (jamais tronqué dans le domaine).
    pub id: String,
    /// Nom d'affichage, sans le `/` initial ajouté par Docker.
    pub name: String,
    /// Image du conteneur.
    pub image: String,
    /// État normalisé.
    pub state: ContainerState,
    /// Statut lisible renvoyé par le moteur (ex. `Up 2 hours`).
    pub status: String,
    /// Date de création (epoch UNIX, secondes) si le moteur la fournit.
    pub created: Option<i64>,
}

/// Détail minimal d'un conteneur, tel qu'affiché par `hormos inspect`.
///
/// Volontairement restreint : **aucune** variable d'environnement, aucun secret,
/// aucune configuration complète.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerDetails {
    /// Identifiant complet.
    pub id: String,
    /// Nom d'affichage, sans le `/` initial.
    pub name: String,
    /// Image du conteneur.
    pub image: String,
    /// État normalisé.
    pub state: ContainerState,
    /// Statut détaillé si le moteur le fournit.
    pub status: Option<String>,
    /// Date de création (chaîne RFC 3339 renvoyée par le moteur).
    pub created: Option<String>,
    /// Nom d'hôte configuré dans le conteneur.
    pub hostname: Option<String>,
    /// Nombre de redémarrages effectués par le moteur.
    pub restart_count: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::ContainerState;

    #[test]
    fn normalizes_known_states() {
        assert_eq!(
            ContainerState::from_engine("running"),
            ContainerState::Running
        );
        assert_eq!(
            ContainerState::from_engine("  Exited "),
            ContainerState::Exited
        );
        assert_eq!(
            ContainerState::from_engine("PAUSED"),
            ContainerState::Paused
        );
    }

    #[test]
    fn keeps_unknown_states_verbatim() {
        let state = ContainerState::from_engine("zombie");
        assert_eq!(state, ContainerState::Other("zombie".into()));
        assert_eq!(state.as_str(), "zombie");
        assert!(!state.is_running());
    }

    #[test]
    fn running_detection() {
        assert!(ContainerState::from_engine("running").is_running());
        assert!(!ContainerState::from_engine("exited").is_running());
    }

    #[test]
    fn serializes_as_a_plain_string() {
        let json = serde_json::to_string(&ContainerState::Running);
        assert_eq!(json.ok().as_deref(), Some("\"running\""));

        let json = serde_json::to_string(&ContainerState::from_engine("zombie"));
        assert_eq!(json.ok().as_deref(), Some("\"zombie\""));
    }
}

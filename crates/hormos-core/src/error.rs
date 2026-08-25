//! Modèle d'erreurs d'Hormos.
//!
//! Les erreurs du cœur sont **indépendantes du moteur** : aucune variante ne
//! transporte de type Bollard, d'URL, d'en-tête HTTP ou de contenu sensible. Les
//! adaptateurs (par exemple `hormos-docker`) traduisent leurs erreurs natives
//! vers cette hiérarchie, en ne conservant qu'un message court et déjà assaini.

use thiserror::Error;

/// Résultat standard d'Hormos.
pub type Result<T> = std::result::Result<T, HormosError>;

/// Erreur métier d'Hormos, classée par cause pour permettre à une interface de
/// choisir un message et un code de sortie pertinents.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HormosError {
    /// Le démon du moteur de conteneurs n'est pas joignable.
    #[error("moteur de conteneurs injoignable : {detail}")]
    DaemonUnavailable {
        /// Détail court et non sensible (ex. chemin du socket).
        detail: String,
    },

    /// Le moteur refuse l'accès (socket non accessible à l'utilisateur courant).
    #[error("accès refusé par le moteur de conteneurs : {detail}")]
    PermissionDenied {
        /// Détail court et non sensible.
        detail: String,
    },

    /// Aucun conteneur ne correspond à la référence demandée.
    #[error("conteneur introuvable : {reference}")]
    ContainerNotFound {
        /// Référence demandée (déjà validée, donc sûre à afficher).
        reference: String,
    },

    /// L'état actuel du conteneur interdit l'opération demandée.
    #[error("conflit d'état : {detail}")]
    Conflict {
        /// Détail court renvoyé par le moteur, assaini.
        detail: String,
    },

    /// L'opération a dépassé le délai maximal accordé.
    #[error("délai dépassé pour l'opération « {operation} » ({seconds} s)")]
    Timeout {
        /// Nom de l'opération (constante interne, jamais une donnée externe).
        operation: &'static str,
        /// Délai accordé, en secondes.
        seconds: u64,
    },

    /// L'entrée fournie par l'utilisateur est invalide.
    #[error("entrée invalide : {reason}")]
    InvalidInput {
        /// Raison du refus, sans réafficher la valeur brute non assainie.
        reason: String,
    },

    /// Le point de terminaison ou le moteur demandé n'est pas supporté.
    #[error("moteur ou point de terminaison non supporté : {detail}")]
    UnsupportedRuntime {
        /// Détail court (ex. schéma d'URL refusé).
        detail: String,
    },

    /// Toute autre erreur renvoyée par le moteur.
    #[error("erreur du moteur de conteneurs : {detail}")]
    RuntimeError {
        /// Message court et assaini.
        detail: String,
    },
}

impl HormosError {
    /// Identifiant stable de la catégorie d'erreur.
    ///
    /// Utilisé par les interfaces pour choisir un code de sortie ou une clé de
    /// message sans dépendre du texte affiché.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::DaemonUnavailable { .. } => ErrorKind::DaemonUnavailable,
            Self::PermissionDenied { .. } => ErrorKind::PermissionDenied,
            Self::ContainerNotFound { .. } => ErrorKind::ContainerNotFound,
            Self::Conflict { .. } => ErrorKind::Conflict,
            Self::Timeout { .. } => ErrorKind::Timeout,
            Self::InvalidInput { .. } => ErrorKind::InvalidInput,
            Self::UnsupportedRuntime { .. } => ErrorKind::UnsupportedRuntime,
            Self::RuntimeError { .. } => ErrorKind::RuntimeError,
        }
    }

    /// Construit une erreur d'entrée invalide.
    #[must_use]
    pub fn invalid_input(reason: impl Into<String>) -> Self {
        Self::InvalidInput {
            reason: reason.into(),
        }
    }

    /// Construit une erreur moteur générique.
    #[must_use]
    pub fn runtime(detail: impl Into<String>) -> Self {
        Self::RuntimeError {
            detail: detail.into(),
        }
    }
}

/// Catégorie d'erreur, stable et exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Voir [`HormosError::DaemonUnavailable`].
    DaemonUnavailable,
    /// Voir [`HormosError::PermissionDenied`].
    PermissionDenied,
    /// Voir [`HormosError::ContainerNotFound`].
    ContainerNotFound,
    /// Voir [`HormosError::Conflict`].
    Conflict,
    /// Voir [`HormosError::Timeout`].
    Timeout,
    /// Voir [`HormosError::InvalidInput`].
    InvalidInput,
    /// Voir [`HormosError::UnsupportedRuntime`].
    UnsupportedRuntime,
    /// Voir [`HormosError::RuntimeError`].
    RuntimeError,
}

#[cfg(test)]
mod tests {
    use super::{ErrorKind, HormosError};

    #[test]
    fn kind_matches_variant() {
        let cases = [
            (
                HormosError::DaemonUnavailable {
                    detail: "socket".into(),
                },
                ErrorKind::DaemonUnavailable,
            ),
            (
                HormosError::ContainerNotFound {
                    reference: "nginx".into(),
                },
                ErrorKind::ContainerNotFound,
            ),
            (
                HormosError::Timeout {
                    operation: "ping",
                    seconds: 5,
                },
                ErrorKind::Timeout,
            ),
            (HormosError::invalid_input("vide"), ErrorKind::InvalidInput),
            (HormosError::runtime("boom"), ErrorKind::RuntimeError),
        ];
        for (error, expected) in cases {
            assert_eq!(error.kind(), expected);
        }
    }

    #[test]
    fn messages_are_short_and_typed() {
        let error = HormosError::ContainerNotFound {
            reference: "missing".into(),
        };
        assert_eq!(error.to_string(), "conteneur introuvable : missing");
    }
}

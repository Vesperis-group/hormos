//! Traduction des erreurs Bollard vers le modèle d'erreurs d'Hormos.
//!
//! Objectifs :
//!
//! - **classer** l'erreur (démon injoignable, permission, introuvable, conflit,
//!   délai dépassé, autre) pour que les interfaces choisissent un message et un
//!   code de sortie pertinents ;
//! - **ne pas fuiter** : ni URL, ni en-tête, ni corps HTTP brut, ni jeton. Seul
//!   un message court est conservé, tronqué et débarrassé de ses caractères de
//!   contrôle (le message d'erreur du moteur peut contenir un nom de conteneur
//!   choisi par un tiers).

use std::error::Error as StdError;
use std::io::ErrorKind as IoErrorKind;

use bollard::errors::Error as BollardError;
use hormos_core::display::sanitize_truncated;
use hormos_core::error::HormosError;

/// Longueur maximale d'un message d'erreur moteur réexposé.
const MAX_DETAIL_LEN: usize = 200;

/// Traduit une erreur Bollard, en tenant compte du contexte de l'appel.
///
/// `endpoint` sert à rendre les erreurs de connexion actionnables ; `reference`
/// est la référence de conteneur **déjà validée** de l'opération, s'il y en a une.
pub(crate) fn map_error(
    error: &BollardError,
    endpoint: &str,
    reference: Option<&str>,
) -> HormosError {
    match error {
        BollardError::DockerResponseServerError {
            status_code,
            message,
        } => from_status(*status_code, message, reference),

        BollardError::SocketNotFoundError(path) => HormosError::DaemonUnavailable {
            detail: format!("socket introuvable : {}", detail(path)),
        },

        BollardError::RequestTimeoutError => HormosError::Timeout {
            operation: "requête Docker",
            seconds: crate::timeouts::CLIENT_SECONDS,
        },

        BollardError::UnsupportedURISchemeError { uri } => HormosError::UnsupportedRuntime {
            detail: format!("schéma non supporté : {}", detail(uri)),
        },

        other => from_transport(other, endpoint),
    }
}

/// Classe une réponse d'erreur du moteur à partir de son code HTTP.
///
/// Fonction pure : testable sans démon.
fn from_status(status_code: u16, message: &str, reference: Option<&str>) -> HormosError {
    match status_code {
        404 => HormosError::ContainerNotFound {
            reference: reference.unwrap_or("(inconnu)").to_owned(),
        },
        401 | 403 => HormosError::PermissionDenied {
            detail: detail(message),
        },
        409 => HormosError::Conflict {
            detail: detail(message),
        },
        400 | 422 => HormosError::invalid_input(detail(message)),
        503 => HormosError::DaemonUnavailable {
            detail: detail(message),
        },
        _ => HormosError::RuntimeError {
            detail: format!("HTTP {status_code} : {}", detail(message)),
        },
    }
}

/// Classe une erreur de transport local en inspectant la chaîne de causes.
fn from_transport(error: &BollardError, endpoint: &str) -> HormosError {
    match io_kind(error) {
        Some(IoErrorKind::PermissionDenied) => HormosError::PermissionDenied {
            detail: format!(
                "accès refusé au socket {} ; \
                 vérifiez que votre utilisateur appartient au groupe « docker »",
                detail(endpoint)
            ),
        },
        Some(IoErrorKind::NotFound) => HormosError::DaemonUnavailable {
            detail: format!("socket introuvable : {}", detail(endpoint)),
        },
        Some(
            IoErrorKind::ConnectionRefused
            | IoErrorKind::ConnectionReset
            | IoErrorKind::ConnectionAborted
            | IoErrorKind::BrokenPipe
            | IoErrorKind::NotConnected,
        ) => HormosError::DaemonUnavailable {
            detail: format!("connexion refusée sur {}", detail(endpoint)),
        },
        Some(IoErrorKind::TimedOut) => HormosError::Timeout {
            operation: "requête Docker",
            seconds: crate::timeouts::CLIENT_SECONDS,
        },
        _ => HormosError::RuntimeError {
            detail: detail(&error.to_string()),
        },
    }
}

/// Cherche une [`std::io::Error`] dans la chaîne de causes.
fn io_kind(error: &(dyn StdError + 'static)) -> Option<IoErrorKind> {
    let mut current: Option<&(dyn StdError + 'static)> = Some(error);
    while let Some(err) = current {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            return Some(io.kind());
        }
        current = err.source();
    }
    None
}

/// Assainit et borne un fragment de message provenant du moteur.
fn detail(value: &str) -> String {
    sanitize_truncated(value.trim(), MAX_DETAIL_LEN)
}

#[cfg(test)]
mod tests {
    use super::{detail, from_status};
    use hormos_core::error::{ErrorKind, HormosError};

    #[test]
    fn classifies_response_status_codes() {
        let cases = [
            (404, ErrorKind::ContainerNotFound),
            (401, ErrorKind::PermissionDenied),
            (403, ErrorKind::PermissionDenied),
            (409, ErrorKind::Conflict),
            (400, ErrorKind::InvalidInput),
            (422, ErrorKind::InvalidInput),
            (503, ErrorKind::DaemonUnavailable),
            (500, ErrorKind::RuntimeError),
            (418, ErrorKind::RuntimeError),
        ];
        for (status, expected) in cases {
            let error = from_status(status, "message", Some("web"));
            assert_eq!(error.kind(), expected, "code {status} mal classé");
        }
    }

    #[test]
    fn not_found_reports_the_validated_reference_only() {
        let error = from_status(404, "No such container: web\u{1b}[2K", Some("web"));
        assert_eq!(
            error,
            HormosError::ContainerNotFound {
                reference: "web".into()
            }
        );
    }

    #[test]
    fn engine_messages_are_sanitized_and_bounded() {
        let hostile = format!("boom\u{1b}[2K\n{}", "x".repeat(500));
        let error = from_status(500, &hostile, None);
        let rendered = error.to_string();
        assert!(!rendered.contains('\u{1b}'), "séquence ANSI conservée");
        assert!(!rendered.contains('\n'), "saut de ligne conservé");
        assert!(rendered.chars().count() < 260, "message non borné");
    }

    #[test]
    fn detail_neutralizes_control_characters() {
        assert_eq!(detail("  a\u{1b}b\n  "), "a\u{fffd}b");
    }
}

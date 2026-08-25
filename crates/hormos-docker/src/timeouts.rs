//! Délais maximaux appliqués à chaque opération Docker.
//!
//! Aucun appel ne peut bloquer indéfiniment. La politique est volontairement
//! **interne et fixe** : pas de système de configuration à ce stade.
//!
//! Les opérations de cycle de vie disposent d'une marge large, car le moteur
//! accorde lui-même un délai de grâce à l'arrêt d'un conteneur (10 s par défaut,
//! puis `SIGKILL`). Le délai de lecture/écriture du client Bollard
//! ([`CLIENT_SECONDS`]) est supérieur à tous les autres : c'est donc toujours le
//! délai d'Hormos qui tranche, jamais un abandon opaque du client HTTP.

/// Délai de lecture/écriture du client Bollard (borne supérieure).
pub const CLIENT_SECONDS: u64 = 120;

/// Négociation de version de l'API à la connexion.
pub const NEGOTIATE_SECONDS: u64 = 5;

/// Vérification de disponibilité du moteur (`ping`).
pub const PING_SECONDS: u64 = 5;

/// Informations système.
pub const INFO_SECONDS: u64 = 10;

/// Listing des conteneurs.
pub const LIST_SECONDS: u64 = 15;

/// Inspection d'un conteneur.
pub const INSPECT_SECONDS: u64 = 10;

/// Démarrage d'un conteneur.
pub const START_SECONDS: u64 = 30;

/// Arrêt d'un conteneur (délai de grâce du moteur inclus).
pub const STOP_SECONDS: u64 = 60;

/// Redémarrage d'un conteneur (arrêt + démarrage).
pub const RESTART_SECONDS: u64 = 90;

#[cfg(test)]
mod tests {
    use super::{
        CLIENT_SECONDS, INFO_SECONDS, INSPECT_SECONDS, LIST_SECONDS, NEGOTIATE_SECONDS,
        PING_SECONDS, RESTART_SECONDS, START_SECONDS, STOP_SECONDS,
    };

    #[test]
    fn every_operation_has_a_bound() {
        let operations = [
            NEGOTIATE_SECONDS,
            PING_SECONDS,
            INFO_SECONDS,
            LIST_SECONDS,
            INSPECT_SECONDS,
            START_SECONDS,
            STOP_SECONDS,
            RESTART_SECONDS,
        ];
        assert!(operations.iter().all(|s| *s > 0));
    }

    #[test]
    fn client_timeout_exceeds_every_operation() {
        let longest = RESTART_SECONDS.max(STOP_SECONDS).max(START_SECONDS);
        assert!(
            CLIENT_SECONDS > longest,
            "le délai du client HTTP doit laisser Hormos trancher en premier"
        );
    }

    #[test]
    fn lifecycle_leaves_room_for_the_engine_grace_period() {
        // Le moteur attend 10 s avant SIGKILL : un arrêt doit pouvoir aboutir.
        const _: () = assert!(STOP_SECONDS >= 30);
        const _: () = assert!(RESTART_SECONDS >= STOP_SECONDS);
    }
}

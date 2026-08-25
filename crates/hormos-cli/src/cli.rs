//! Définition de la ligne de commande.
//!
//! Le parsing est isolé du reste : `main` se contente d'exécuter la commande
//! résolue. Aucune option ne permet de désigner un moteur distant — c'est un
//! choix de conception, pas un oubli (voir `docs/security-model.md`).

use clap::{Parser, Subcommand};

/// Control plane conteneurs local-first, orienté sécurité.
#[derive(Debug, Parser)]
#[command(
    name = "hormos",
    version,
    about = "Control plane conteneurs local-first, orienté sécurité.",
    long_about = "Hormos pilote le moteur de conteneurs local via son socket Unix.\n\
                  Aucun transport distant n'est supporté dans cette version.",
    propagate_version = true
)]
pub struct Cli {
    /// Commande à exécuter.
    #[command(subcommand)]
    pub command: Command,
}

/// Commandes disponibles.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Affiche les informations du moteur de conteneurs local.
    Info {
        /// Sortie JSON, stable et scriptable.
        #[arg(long)]
        json: bool,
    },

    /// Liste les conteneurs.
    Ps {
        /// Inclut les conteneurs arrêtés.
        #[arg(short, long)]
        all: bool,

        /// Sortie JSON, stable et scriptable.
        #[arg(long)]
        json: bool,
    },

    /// Affiche le détail d'un conteneur.
    Inspect {
        /// Nom ou identifiant du conteneur.
        reference: String,

        /// Sortie JSON, stable et scriptable.
        #[arg(long)]
        json: bool,
    },

    /// Démarre un conteneur (idempotent).
    Start {
        /// Nom ou identifiant du conteneur.
        reference: String,
    },

    /// Arrête un conteneur (idempotent).
    Stop {
        /// Nom ou identifiant du conteneur.
        reference: String,
    },

    /// Redémarre un conteneur.
    Restart {
        /// Nom ou identifiant du conteneur.
        reference: String,
    },
}

impl Command {
    /// Référence de conteneur portée par la commande, s'il y en a une.
    ///
    /// Permet de valider l'entrée **avant** toute connexion au moteur.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::Info { .. } | Self::Ps { .. } => None,
            Self::Inspect { reference, .. }
            | Self::Start { reference }
            | Self::Stop { reference }
            | Self::Restart { reference } => Some(reference),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Command};

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_ps_flags() {
        let cli = Cli::try_parse_from(["hormos", "ps", "--all", "--json"]);
        assert!(matches!(
            cli.map(|c| c.command),
            Ok(Command::Ps {
                all: true,
                json: true
            })
        ));
    }

    #[test]
    fn ps_defaults_to_running_containers_only() {
        let cli = Cli::try_parse_from(["hormos", "ps"]);
        assert!(matches!(
            cli.map(|c| c.command),
            Ok(Command::Ps {
                all: false,
                json: false
            })
        ));
    }

    #[test]
    fn lifecycle_commands_require_a_reference() {
        for command in ["start", "stop", "restart", "inspect"] {
            assert!(
                Cli::try_parse_from(["hormos", command]).is_err(),
                "« {command} » accepte une référence manquante"
            );
        }
    }

    #[test]
    fn no_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["hormos"]).is_err());
    }

    #[test]
    fn there_is_no_remote_endpoint_option() {
        // Garde-fou : aucune option ne doit permettre de viser un moteur distant.
        for flag in ["--host", "--remote", "--tcp", "--url", "--context"] {
            assert!(
                Cli::try_parse_from(["hormos", "info", flag, "tcp://10.0.0.1:2375"]).is_err(),
                "option de transport distant acceptée : {flag}"
            );
        }
    }

    #[test]
    fn only_container_commands_carry_a_reference() {
        let reference = |args: [&str; 3]| {
            Cli::try_parse_from(args)
                .ok()
                .and_then(|c| c.command.reference().map(ToOwned::to_owned))
        };
        assert_eq!(
            reference(["hormos", "inspect", "web"]).as_deref(),
            Some("web")
        );
        assert_eq!(
            reference(["hormos", "start", "web"]).as_deref(),
            Some("web")
        );
        assert_eq!(reference(["hormos", "stop", "web"]).as_deref(), Some("web"));
        assert_eq!(
            reference(["hormos", "restart", "web"]).as_deref(),
            Some("web")
        );

        let listing = Cli::try_parse_from(["hormos", "ps"])
            .ok()
            .and_then(|c| c.command.reference().map(ToOwned::to_owned));
        assert_eq!(listing, None);
    }
}

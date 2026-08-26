//! Définition de la ligne de commande.
//!
//! Le parsing est isolé du reste : `main` se contente d'exécuter la commande
//! résolue. Aucune option ne permet de désigner un moteur distant — c'est un
//! choix de conception, pas un oubli (voir `docs/security-model.md`).

use clap::{Parser, Subcommand};
use hormos_core::logs::LogTail;

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
    ///
    /// Absente, elle vaut [`Command::Tui`] : `hormos` seul ouvre l'interface
    /// terminal, comme `htop` ou `lazygit`.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Commandes disponibles.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Ouvre l'interface terminal interactive (comportement par défaut).
    Tui,

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

    /// Affiche le journal d'un conteneur.
    Logs {
        /// Nom ou identifiant du conteneur.
        reference: String,

        /// Continue à suivre le journal jusqu'à `Ctrl+C`.
        #[arg(short, long)]
        follow: bool,

        /// Nombre de lignes d'historique : un entier, ou `all`.
        #[arg(long, default_value = "all", value_parser = parse_tail)]
        tail: LogTail,

        /// Préfixe chaque ligne de l'horodatage fourni par le moteur.
        #[arg(long)]
        timestamps: bool,
    },

    /// Suit les événements du moteur de conteneurs.
    Events {
        /// Sortie NDJSON : un objet complet par ligne, lisible au fil de l'eau.
        #[arg(long)]
        json: bool,
    },
}

/// Valide `--tail` dès l'analyse de la ligne de commande.
///
/// Une valeur hors bornes est ainsi refusée avant toute connexion au moteur, et
/// avec le code de sortie d'usage.
fn parse_tail(value: &str) -> std::result::Result<LogTail, String> {
    LogTail::parse(value).map_err(|error| error.to_string())
}

impl Command {
    /// Référence de conteneur portée par la commande, s'il y en a une.
    ///
    /// Permet de valider l'entrée **avant** toute connexion au moteur.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::Tui | Self::Info { .. } | Self::Ps { .. } | Self::Events { .. } => None,
            Self::Inspect { reference, .. }
            | Self::Start { reference }
            | Self::Stop { reference }
            | Self::Restart { reference }
            | Self::Logs { reference, .. } => Some(reference),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use hormos_core::logs::LogTail;

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
            Ok(Some(Command::Ps {
                all: true,
                json: true
            }))
        ));
    }

    #[test]
    fn ps_defaults_to_running_containers_only() {
        let cli = Cli::try_parse_from(["hormos", "ps"]);
        assert!(matches!(
            cli.map(|c| c.command),
            Ok(Some(Command::Ps {
                all: false,
                json: false
            }))
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
    fn no_subcommand_opens_the_terminal_interface() {
        assert!(matches!(
            Cli::try_parse_from(["hormos"]).map(|c| c.command),
            Ok(None)
        ));
        assert!(matches!(
            Cli::try_parse_from(["hormos", "tui"]).map(|c| c.command),
            Ok(Some(Command::Tui))
        ));
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
                .and_then(|c| c.command)
                .and_then(|command| command.reference().map(ToOwned::to_owned))
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
            .and_then(|c| c.command)
            .and_then(|command| command.reference().map(ToOwned::to_owned));
        assert_eq!(listing, None);

        assert_eq!(
            reference(["hormos", "logs", "web"]).as_deref(),
            Some("web"),
            "« logs » doit être validé avant toute connexion"
        );
    }

    #[test]
    fn logs_defaults_to_the_whole_history_without_following() {
        assert!(matches!(
            Cli::try_parse_from(["hormos", "logs", "web"]).map(|c| c.command),
            Ok(Some(Command::Logs {
                follow: false,
                tail: LogTail::All,
                timestamps: false,
                ..
            }))
        ));
    }

    #[test]
    fn logs_accepts_its_flags() {
        assert!(matches!(
            Cli::try_parse_from([
                "hormos",
                "logs",
                "web",
                "-f",
                "--tail",
                "20",
                "--timestamps"
            ])
            .map(|c| c.command),
            Ok(Some(Command::Logs {
                follow: true,
                tail: LogTail::Lines(20),
                timestamps: true,
                ..
            }))
        ));
    }

    #[test]
    fn logs_rejects_an_out_of_range_tail_at_parse_time() {
        for value in ["-1", "abc", "999999999", ""] {
            assert!(
                Cli::try_parse_from(["hormos", "logs", "web", "--tail", value]).is_err(),
                "--tail {value} accepté à tort"
            );
        }
    }

    #[test]
    fn logs_requires_a_reference() {
        assert!(Cli::try_parse_from(["hormos", "logs"]).is_err());
    }

    #[test]
    fn events_takes_no_reference_and_defaults_to_a_table() {
        assert!(matches!(
            Cli::try_parse_from(["hormos", "events"]).map(|c| c.command),
            Ok(Some(Command::Events { json: false }))
        ));
        assert!(matches!(
            Cli::try_parse_from(["hormos", "events", "--json"]).map(|c| c.command),
            Ok(Some(Command::Events { json: true }))
        ));
    }
}

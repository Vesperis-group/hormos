//! Binaire `hormos`.
//!
//! Point de composition unique : c'est le seul endroit de la CLI qui nomme une
//! implémentation concrète du moteur (`hormos_docker::DockerRuntime`). Toute la
//! logique passe par [`ContainerService`], donc par le trait
//! [`ContainerRuntime`](hormos_core::runtime::ContainerRuntime).
//!
//! Conventions de sortie :
//!
//! - le **résultat** va sur `stdout`, les **erreurs** sur `stderr` ;
//! - le code de sortie est dérivé de la catégorie d'erreur, afin qu'un script
//!   puisse distinguer « conteneur introuvable » de « démon injoignable » sans
//!   analyser un message.

mod cli;
mod output;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use hormos_core::error::{ErrorKind, HormosError, Result};
use hormos_core::service::ContainerService;
use hormos_docker::DockerRuntime;

use crate::cli::{Cli, Command};

/// Succès.
const EXIT_OK: u8 = 0;
/// Erreur non classée.
const EXIT_FAILURE: u8 = 1;
/// Entrée invalide (aligné sur le code d'usage de clap).
const EXIT_INVALID_INPUT: u8 = 2;
/// Moteur injoignable.
const EXIT_DAEMON_UNAVAILABLE: u8 = 3;
/// Accès refusé.
const EXIT_PERMISSION_DENIED: u8 = 4;
/// Conteneur introuvable.
const EXIT_NOT_FOUND: u8 = 5;
/// Conflit d'état.
const EXIT_CONFLICT: u8 = 6;
/// Délai dépassé.
const EXIT_TIMEOUT: u8 = 7;
/// Moteur ou transport non supporté.
const EXIT_UNSUPPORTED: u8 = 8;

/// Un seul thread suffit : la CLI exécute une opération, puis se termine.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(error) => {
            eprintln!("hormos : {error}");
            ExitCode::from(exit_code(error.kind()))
        }
    }
}

/// Code de sortie associé à une catégorie d'erreur.
const fn exit_code(kind: ErrorKind) -> u8 {
    match kind {
        ErrorKind::InvalidInput => EXIT_INVALID_INPUT,
        ErrorKind::DaemonUnavailable => EXIT_DAEMON_UNAVAILABLE,
        ErrorKind::PermissionDenied => EXIT_PERMISSION_DENIED,
        ErrorKind::ContainerNotFound => EXIT_NOT_FOUND,
        ErrorKind::Conflict => EXIT_CONFLICT,
        ErrorKind::Timeout => EXIT_TIMEOUT,
        ErrorKind::UnsupportedRuntime => EXIT_UNSUPPORTED,
        ErrorKind::RuntimeError => EXIT_FAILURE,
    }
}

async fn run(cli: Cli) -> Result<()> {
    // Sans sous-commande, `hormos` ouvre l'interface terminal.
    let command = cli.command.unwrap_or(Command::Tui);

    // La validation précède toute connexion : une référence invalide échoue sans
    // même ouvrir le socket Docker.
    if let Some(reference) = command.reference() {
        hormos_core::ContainerRef::new(reference)?;
    }
    // Le contrôle du TTY précède lui aussi la connexion : rediriger la sortie
    // d'`hormos` ne doit pas toucher au moteur.
    if matches!(command, Command::Tui) && !std::io::stdout().is_terminal() {
        return Err(HormosError::invalid_input(
            "l'interface terminal exige un terminal interactif ; utilisez « hormos ps »",
        ));
    }

    let service = connect().await?;
    match command {
        Command::Tui => hormos_tui::run(service).await,
        Command::Info { json } => {
            let info = service.system_info().await?;
            emit(json, &info, || output::render_info(&info))
        }
        Command::Ps { all, json } => {
            let containers = service.list_containers(all).await?;
            emit(json, &containers, || output::render_list(&containers))
        }
        Command::Inspect { reference, json } => {
            let details = service.inspect_container(&reference).await?;
            emit(json, &details, || output::render_details(&details))
        }
        Command::Start { reference } => {
            let started = service.start_container(&reference).await?;
            println!("{}", started.as_str());
            Ok(())
        }
        Command::Stop { reference } => {
            let stopped = service.stop_container(&reference).await?;
            println!("{}", stopped.as_str());
            Ok(())
        }
        Command::Restart { reference } => {
            let restarted = service.restart_container(&reference).await?;
            println!("{}", restarted.as_str());
            Ok(())
        }
    }
}

/// Construit le service au-dessus du moteur Docker local.
async fn connect() -> Result<ContainerService> {
    let runtime = DockerRuntime::connect().await?;
    Ok(ContainerService::new(Arc::new(runtime)))
}

/// Écrit le résultat sur `stdout`, en JSON ou en texte.
fn emit<T: serde::Serialize>(json: bool, value: &T, text: impl FnOnce() -> String) -> Result<()> {
    if json {
        let rendered = output::to_json(value)
            .map_err(|error| HormosError::runtime(format!("sérialisation JSON : {error}")))?;
        println!("{rendered}");
    } else {
        println!("{}", text());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use hormos_core::error::ErrorKind;

    use super::{
        EXIT_CONFLICT, EXIT_DAEMON_UNAVAILABLE, EXIT_FAILURE, EXIT_INVALID_INPUT, EXIT_NOT_FOUND,
        EXIT_OK, EXIT_PERMISSION_DENIED, EXIT_TIMEOUT, EXIT_UNSUPPORTED, exit_code,
    };

    #[test]
    fn every_error_kind_has_a_distinct_exit_code() {
        let kinds = [
            ErrorKind::InvalidInput,
            ErrorKind::DaemonUnavailable,
            ErrorKind::PermissionDenied,
            ErrorKind::ContainerNotFound,
            ErrorKind::Conflict,
            ErrorKind::Timeout,
            ErrorKind::UnsupportedRuntime,
        ];
        let mut codes: Vec<u8> = kinds.iter().copied().map(exit_code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), kinds.len(), "codes de sortie ambigus");
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(ErrorKind::InvalidInput), EXIT_INVALID_INPUT);
        assert_eq!(
            exit_code(ErrorKind::DaemonUnavailable),
            EXIT_DAEMON_UNAVAILABLE
        );
        assert_eq!(
            exit_code(ErrorKind::PermissionDenied),
            EXIT_PERMISSION_DENIED
        );
        assert_eq!(exit_code(ErrorKind::ContainerNotFound), EXIT_NOT_FOUND);
        assert_eq!(exit_code(ErrorKind::Conflict), EXIT_CONFLICT);
        assert_eq!(exit_code(ErrorKind::Timeout), EXIT_TIMEOUT);
        assert_eq!(exit_code(ErrorKind::UnsupportedRuntime), EXIT_UNSUPPORTED);
        assert_eq!(exit_code(ErrorKind::RuntimeError), EXIT_FAILURE);
    }

    #[test]
    fn no_error_maps_to_success() {
        let kinds = [
            ErrorKind::InvalidInput,
            ErrorKind::DaemonUnavailable,
            ErrorKind::PermissionDenied,
            ErrorKind::ContainerNotFound,
            ErrorKind::Conflict,
            ErrorKind::Timeout,
            ErrorKind::UnsupportedRuntime,
            ErrorKind::RuntimeError,
        ];
        assert!(kinds.iter().copied().map(exit_code).all(|c| c != EXIT_OK));
    }
}

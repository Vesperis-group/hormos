//! Interface terminal d'Hormos.
//!
//! « One engine. Every interface. » : le TUI ne contient **aucune** logique
//! Docker. Il reçoit un [`ContainerService`] déjà construit et ne connaît, comme
//! la CLI, que le domaine d'`hormos-core`. Il ne dépend donc ni de `bollard`, ni
//! de `hormos-docker`.
//!
//! # Organisation
//!
//! - [`app`] : l'état et ses transitions, purs et testables sans terminal ;
//! - `event` : la lecture du clavier, sur un fil dédié, vers un canal borné ;
//! - `terminal` : la prise et la restitution du terminal ;
//! - `ui` : le rendu, qui assainit toute chaîne venue du moteur.
//!
//! # Boucle
//!
//! Un seul point de synchronisation : la boucle consomme un [`Message`], le
//! passe à [`App::update`], exécute l'éventuelle [`Command`] renvoyée **hors du
//! rendu**, puis redessine. Il n'y a pas de sondage périodique : Hormos
//! n'interroge le moteur que sur une action explicite de l'utilisateur, ou juste
//! après une action de cycle de vie.

pub mod app;
mod event;
mod terminal;
mod ui;

use hormos_core::error::{HormosError, Result};
use hormos_core::service::ContainerService;
use tokio::sync::mpsc::{self, Sender};

pub use crate::app::{App, Command, Message, Mode, Severity, Status, Verb};
pub use crate::ui::{MIN_HEIGHT, MIN_WIDTH};

use crate::terminal::TerminalGuard;

/// Capacité du canal d'événements.
///
/// Borné volontairement : si l'interface prend du retard (moteur lent, rafale de
/// touches), la lecture du clavier ralentit au lieu de faire grossir une file
/// sans limite.
const CHANNEL_CAPACITY: usize = 64;

/// Ouvre l'interface terminal et rend la main à la sortie de l'utilisateur.
///
/// # Errors
///
/// Renvoie [`HormosError::RuntimeError`] si le terminal ne peut pas être pris ou
/// dessiné. Les erreurs du moteur, elles, ne terminent pas le TUI : elles sont
/// affichées, et l'utilisateur peut réessayer.
pub async fn run(service: ContainerService) -> Result<()> {
    let mut terminal = TerminalGuard::new().map_err(terminal_error)?;
    let (sender, mut receiver) = mpsc::channel(CHANNEL_CAPACITY);
    // Détruit avant `terminal` (ordre inverse de déclaration) : le fil de lecture
    // est donc arrêté avant que le terminal ne soit rendu à l'utilisateur.
    let _input = event::spawn(sender.clone());

    let mut app = App::new();
    dispatch(
        &service,
        &sender,
        Command::Refresh {
            all: app.show_all(),
        },
    );
    terminal
        .draw(|frame| ui::render(&app, frame))
        .map_err(terminal_error)?;

    while let Some(message) = receiver.recv().await {
        if let Some(command) = app.update(message) {
            dispatch(&service, &sender, command);
        }
        if app.should_quit() {
            break;
        }
        terminal
            .draw(|frame| ui::render(&app, frame))
            .map_err(terminal_error)?;
    }
    Ok(())
}

/// Exécute une commande **hors** du rendu, et publie son résultat.
///
/// Chaque commande part dans sa propre tâche : l'interface reste réactive
/// pendant qu'un `stop` consomme son délai de grâce.
fn dispatch(service: &ContainerService, sender: &Sender<Message>, command: Command) {
    let service = service.clone();
    let sender = sender.clone();
    tokio::spawn(async move {
        let message = execute(&service, command).await;
        // Un échec d'envoi signifie que le TUI est déjà terminé : il n'y a rien
        // à signaler.
        let _ = sender.send(message).await;
    });
}

/// Traduit une commande en appel au service, puis en message.
async fn execute(service: &ContainerService, command: Command) -> Message {
    match command {
        Command::Refresh { all } => Message::Containers(service.list_containers(all).await),
        Command::Inspect { id } => {
            Message::Details(service.inspect_container(&id).await.map(Box::new))
        }
        Command::Act { id, verb } => {
            let outcome = match verb {
                Verb::Start => service.start_container(&id).await,
                Verb::Stop => service.stop_container(&id).await,
                Verb::Restart => service.restart_container(&id).await,
            };
            Message::Acted {
                id,
                verb,
                outcome: outcome.map(|_| ()),
            }
        }
    }
}

fn terminal_error(error: std::io::Error) -> HormosError {
    HormosError::runtime(format!("terminal indisponible : {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hormos_core::domain::ContainerState;
    use hormos_core::error::HormosError;
    use hormos_core::mock::{Call, MockRuntime};
    use hormos_core::service::ContainerService;

    use super::{Command, Message, Verb, execute};

    fn service(mock: Arc<MockRuntime>) -> ContainerService {
        ContainerService::new(mock)
    }

    #[tokio::test]
    async fn refresh_asks_the_service_for_the_requested_scope() {
        let mock = Arc::new(MockRuntime::new());
        let message = execute(&service(Arc::clone(&mock)), Command::Refresh { all: true }).await;

        assert!(
            matches!(message, Message::Containers(Ok(ref containers)) if containers.len() == 2)
        );
        assert_eq!(mock.calls(), vec![Call::List(true)]);
    }

    #[tokio::test]
    async fn each_verb_reaches_the_matching_use_case() {
        let cases = [
            (Verb::Start, Call::Start("web".into())),
            (Verb::Stop, Call::Stop("web".into())),
            (Verb::Restart, Call::Restart("web".into())),
        ];
        for (verb, expected) in cases {
            let mock = Arc::new(MockRuntime::new());
            let message = execute(
                &service(Arc::clone(&mock)),
                Command::Act {
                    id: "web".into(),
                    verb,
                },
            )
            .await;

            assert!(matches!(
                message,
                Message::Acted {
                    verb: applied,
                    outcome: Ok(()),
                    ..
                } if applied == verb
            ));
            assert_eq!(mock.calls(), vec![expected]);
        }
    }

    #[tokio::test]
    async fn inspect_returns_details_for_the_selected_container() {
        let mock = Arc::new(MockRuntime::new());
        let message = execute(
            &service(Arc::clone(&mock)),
            Command::Inspect { id: "web".into() },
        )
        .await;

        assert!(
            matches!(message, Message::Details(Ok(ref details)) if details.state == ContainerState::Running)
        );
        assert_eq!(mock.calls(), vec![Call::Inspect("web".into())]);
    }

    #[tokio::test]
    async fn engine_failures_become_messages_not_crashes() {
        let error = HormosError::DaemonUnavailable {
            detail: "/var/run/docker.sock".into(),
        };
        let mock = Arc::new(MockRuntime::failing(error.clone()));
        let message = execute(&service(mock), Command::Refresh { all: false }).await;

        assert_eq!(message, Message::Containers(Err(error)));
    }
}

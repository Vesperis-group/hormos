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
//! Deux sources, une seule boucle. Les touches et les résultats d'appels
//! ponctuels arrivent par le canal de *contrôle* ; les flux — journal ou
//! événements — par un canal séparé. La sélection est **biaisée** dans cet
//! ordre : contrôle, rendu, flux. Un conteneur qui écrit sans discontinuer ne
//! peut donc ni retarder une touche, ni empêcher l'écran de se redessiner.
//!
//! Le rendu n'est pas déclenché par chaque message mais par une horloge : sous
//! une rafale, l'écran est redessiné au plus une fois par [`RENDER_INTERVAL`],
//! au lieu d'une fois par ligne reçue.
//!
//! Les deux canaux sont **bornés**. Un flux plus rapide que l'affichage attend
//! sur son envoi ; c'est cette attente qui remonte la contre-pression jusqu'au
//! moteur, plutôt que de laisser une file grossir sans limite.

pub mod app;
mod event;
pub mod stream;
mod terminal;
mod ui;

use std::time::Duration;

use hormos_core::error::{HormosError, Result};
use hormos_core::logs::{LogFramer, LogSource};
use hormos_core::service::ContainerService;
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinHandle;

pub use crate::app::{App, Command, Message, Mode, Severity, Status, Verb};
pub use crate::ui::{MIN_HEIGHT, MIN_WIDTH};

use crate::stream::{LogLine, MAX_LINE_BYTES};
use crate::terminal::TerminalGuard;

/// Capacité du canal d'événements.
///
/// Borné volontairement : si l'interface prend du retard (moteur lent, rafale de
/// touches), la lecture du clavier ralentit au lieu de faire grossir une file
/// sans limite.
const CHANNEL_CAPACITY: usize = 64;

/// Capacité du canal de flux.
///
/// Plus large que celui de contrôle — un fragment de journal peut porter
/// plusieurs lignes — mais borné pour la même raison : au-delà, la tâche de flux
/// attend, et le moteur ralentit avec elle.
const STREAM_CAPACITY: usize = 256;

/// Période minimale entre deux rendus.
///
/// Environ soixante images par seconde : au-delà, l'œil ne suit plus et chaque
/// dessin coûte un écran complet.
const RENDER_INTERVAL: Duration = Duration::from_millis(16);

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
    let (stream_sender, mut stream_receiver) = mpsc::channel(STREAM_CAPACITY);
    // Détruit avant `terminal` (ordre inverse de déclaration) : le fil de lecture
    // est donc arrêté avant que le terminal ne soit rendu à l'utilisateur. À cet
    // instant `receiver` est encore vivant, donc le canal encore ouvert : c'est
    // pourquoi l'envoi côté fil doit rester annulable, quelle que soit la sortie
    // — `q`, erreur de rendu, ou fin du canal.
    let _input = event::spawn(sender.clone());

    let mut app = App::new();
    // Détruit à la sortie de `run` : la requête HTTP du flux est refermée avec
    // lui, sans quoi une tâche continuerait de lire un socket que plus personne
    // ne regarde.
    let mut stream = StreamTask::default();
    dispatch(
        &service,
        &sender,
        &stream_sender,
        &mut stream,
        Command::Refresh {
            all: app.show_all(),
        },
    );
    draw(&mut terminal, &mut app)?;

    let mut ticker = tokio::time::interval(RENDER_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut dirty = false;

    loop {
        // L'ordre est significatif : le clavier d'abord, le rendu ensuite, le
        // flux en dernier. Les deux `recv()` portent sur des canaux dont un
        // émetteur reste vivant ici, donc leurs branches ne se désactivent
        // jamais : la sélection ne peut pas se retrouver sans branche active.
        let message = tokio::select! {
            biased;
            Some(message) = receiver.recv() => message,
            _ = ticker.tick(), if dirty => {
                draw(&mut terminal, &mut app)?;
                dirty = false;
                continue;
            }
            Some(message) = stream_receiver.recv() => message,
        };

        if let Some(command) = app.update(message) {
            dispatch(&service, &sender, &stream_sender, &mut stream, command);
        }
        if app.should_quit() {
            break;
        }
        dirty = true;
    }
    Ok(())
}

/// Dessine l'écran après avoir transmis à l'état la hauteur réelle disponible.
fn draw(terminal: &mut TerminalGuard, app: &mut App) -> Result<()> {
    let area = terminal.area().map_err(terminal_error)?;
    app.set_viewport(ui::stream_viewport(area));
    terminal
        .draw(|frame| ui::render(app, frame))
        .map_err(terminal_error)
}

/// Flux actif, s'il y en a un.
///
/// Un seul à la fois : le TUI n'affiche qu'un panneau, et deux abonnements
/// simultanés doubleraient la charge sans que rien ne s'affiche de plus.
#[derive(Debug, Default)]
struct StreamTask {
    handle: Option<JoinHandle<()>>,
}

impl StreamTask {
    /// Interrompt le flux en cours.
    ///
    /// `abort()` referme la requête HTTP et débloque aussi une tâche arrêtée sur
    /// un envoi en contre-pression : sans cela, un flux plus rapide que
    /// l'affichage ne se terminerait jamais.
    fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }

    fn replace(&mut self, handle: JoinHandle<()>) {
        self.stop();
        self.handle = Some(handle);
    }
}

impl Drop for StreamTask {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Exécute une commande **hors** du rendu, et publie son résultat.
///
/// Chaque commande part dans sa propre tâche : l'interface reste réactive
/// pendant qu'un `stop` consomme son délai de grâce.
fn dispatch(
    service: &ContainerService,
    sender: &Sender<Message>,
    stream_sender: &Sender<Message>,
    stream: &mut StreamTask,
    command: Command,
) {
    match command {
        Command::CloseStream => stream.stop(),
        Command::OpenLogs {
            id,
            options,
            generation,
        } => {
            let service = service.clone();
            let sender = stream_sender.clone();
            stream.replace(tokio::spawn(async move {
                pump_logs(&service, &sender, &id, &options, generation).await;
            }));
        }
        Command::OpenEvents { generation } => {
            let service = service.clone();
            let sender = stream_sender.clone();
            stream.replace(tokio::spawn(async move {
                pump_events(&service, &sender, generation).await;
            }));
        }
        other => {
            let service = service.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let message = execute(&service, other).await;
                // Un échec d'envoi signifie que le TUI est déjà terminé : il n'y
                // a rien à signaler.
                let _ = sender.send(message).await;
            });
        }
    }
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
        // Traitées par `dispatch`, qui ne les fait pas passer par ici.
        Command::OpenLogs { .. } | Command::OpenEvents { .. } | Command::CloseStream => {
            Message::Resized
        }
    }
}

/// Lit un journal jusqu'à sa fin, à son échec, ou à l'interruption de la tâche.
///
/// Un découpeur par sortie : `stdout` et `stderr` arrivent entrelacés, et
/// recoller leurs fragments produirait des lignes mélangées et de l'UTF-8
/// invalide.
async fn pump_logs(
    service: &ContainerService,
    sender: &Sender<Message>,
    id: &str,
    options: &hormos_core::logs::LogOptions,
    generation: u64,
) {
    let mut stream = match service.container_logs(id, options) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = sender.send(ended(generation, Err(error))).await;
            return;
        }
    };

    let mut out = LogFramer::new(MAX_LINE_BYTES);
    let mut err = LogFramer::new(MAX_LINE_BYTES);
    let outcome = loop {
        match stream.next().await {
            None => break Ok(()),
            Some(Err(error)) => break Err(error),
            Some(Ok(chunk)) => {
                let source = chunk.source;
                let framer = if source == LogSource::Stderr {
                    &mut err
                } else {
                    &mut out
                };
                let mut lines = Vec::new();
                framer.push(&chunk.data, |text| lines.push(LogLine::new(source, text)));
                if !lines.is_empty()
                    && sender
                        .send(Message::Logs { generation, lines })
                        .await
                        .is_err()
                {
                    return;
                }
            }
        }
    };

    // Une ligne restée sans retour à la ligne final est tout de même montrée :
    // c'est souvent celle qui explique l'arrêt.
    let mut lines = Vec::new();
    lines.extend(
        out.flush()
            .map(|text| LogLine::new(LogSource::Stdout, text)),
    );
    lines.extend(
        err.flush()
            .map(|text| LogLine::new(LogSource::Stderr, text)),
    );
    if !lines.is_empty() {
        let _ = sender.send(Message::Logs { generation, lines }).await;
    }
    let _ = sender.send(ended(generation, outcome)).await;
}

/// Suit les événements du moteur jusqu'à l'interruption de la tâche.
async fn pump_events(service: &ContainerService, sender: &Sender<Message>, generation: u64) {
    let mut stream = match service.runtime_events() {
        Ok(stream) => stream,
        Err(error) => {
            let _ = sender.send(ended(generation, Err(error))).await;
            return;
        }
    };

    let outcome = loop {
        match stream.next().await {
            None => break Ok(()),
            Some(Err(error)) => break Err(error),
            Some(Ok(event)) => {
                let message = Message::Event {
                    generation,
                    event: Box::new(event),
                };
                if sender.send(message).await.is_err() {
                    return;
                }
            }
        }
    };
    let _ = sender.send(ended(generation, outcome)).await;
}

const fn ended(generation: u64, outcome: Result<()>) -> Message {
    Message::StreamEnded {
        generation,
        outcome,
    }
}

fn terminal_error(error: std::io::Error) -> HormosError {
    HormosError::runtime(format!("terminal indisponible : {error}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::sync::Arc;

    use hormos_core::domain::ContainerState;
    use hormos_core::error::HormosError;
    use hormos_core::mock::{Call, MockRuntime};
    use hormos_core::service::ContainerService;

    use std::time::Duration;

    use hormos_core::logs::{LogChunk, LogOptions, LogSource};
    use tokio::sync::mpsc;

    use super::{Command, Message, Verb, execute, pump_events, pump_logs};

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

    // ------------------------------------------------------------------ flux

    /// Échéance stricte : un test de flux ne doit jamais pouvoir suspendre la
    /// suite entière s'il régresse.
    const DEADLINE: Duration = Duration::from_secs(5);

    fn chunk(source: LogSource, data: &[u8]) -> LogChunk {
        LogChunk::new(source, data.to_vec())
    }

    /// Collecte les messages jusqu'à la fin du flux, sous échéance.
    async fn drain(mut receiver: mpsc::Receiver<Message>) -> Vec<Message> {
        let collected = async {
            let mut messages = Vec::new();
            while let Some(message) = receiver.recv().await {
                let last = matches!(message, Message::StreamEnded { .. });
                messages.push(message);
                if last {
                    break;
                }
            }
            messages
        };
        tokio::time::timeout(DEADLINE, collected)
            .await
            .expect("le flux ne s'est pas terminé dans le délai")
    }

    fn texts(messages: &[Message]) -> Vec<(LogSource, String)> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Logs { lines, .. } => Some(lines),
                _ => None,
            })
            .flatten()
            .map(|line| (line.source, line.text.clone()))
            .collect()
    }

    #[tokio::test]
    async fn a_log_is_cut_into_lines_across_fragments() {
        let mock = Arc::new(MockRuntime::new().with_logs(vec![
            Ok(chunk(LogSource::Stdout, b"bon")),
            Ok(chunk(LogSource::Stdout, b"jour\nau revoir\n")),
        ]));
        let (sender, receiver) = mpsc::channel(16);

        pump_logs(&service(mock), &sender, "web", &LogOptions::new(), 7).await;
        drop(sender);
        let messages = drain(receiver).await;

        assert_eq!(
            texts(&messages),
            vec![
                (LogSource::Stdout, "bonjour".to_owned()),
                (LogSource::Stdout, "au revoir".to_owned()),
            ]
        );
        assert!(messages.iter().all(|message| match message {
            Message::Logs { generation, .. } | Message::StreamEnded { generation, .. } =>
                *generation == 7,
            _ => false,
        }));
    }

    #[tokio::test]
    async fn the_two_outputs_keep_their_own_framing() {
        // Fragments entrelacés : recoller `stdout` et `stderr` produirait
        // « bonsoir » et « bonjour » mélangés.
        let mock = Arc::new(MockRuntime::new().with_logs(vec![
            Ok(chunk(LogSource::Stdout, b"bon")),
            Ok(chunk(LogSource::Stderr, b"pan")),
            Ok(chunk(LogSource::Stdout, b"jour\n")),
            Ok(chunk(LogSource::Stderr, b"ne\n")),
        ]));
        let (sender, receiver) = mpsc::channel(16);

        pump_logs(&service(mock), &sender, "web", &LogOptions::new(), 1).await;
        drop(sender);

        assert_eq!(
            texts(&drain(receiver).await),
            vec![
                (LogSource::Stdout, "bonjour".to_owned()),
                (LogSource::Stderr, "panne".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn an_unterminated_last_line_is_still_shown() {
        let mock = Arc::new(MockRuntime::new().with_logs(vec![Ok(chunk(
            LogSource::Stdout,
            b"sans retour a la ligne",
        ))]));
        let (sender, receiver) = mpsc::channel(16);

        pump_logs(&service(mock), &sender, "web", &LogOptions::new(), 1).await;
        drop(sender);
        let messages = drain(receiver).await;

        assert_eq!(
            texts(&messages),
            vec![(LogSource::Stdout, "sans retour a la ligne".to_owned())]
        );
        assert!(matches!(
            messages.last(),
            Some(Message::StreamEnded {
                outcome: Ok(()),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn a_failure_arrives_after_the_lines_that_preceded_it() {
        let error = HormosError::runtime("le moteur a coupé");
        let mock = Arc::new(MockRuntime::new().with_logs(vec![
            Ok(chunk(LogSource::Stdout, b"avant\n")),
            Err(error.clone()),
        ]));
        let (sender, receiver) = mpsc::channel(16);

        pump_logs(&service(mock), &sender, "web", &LogOptions::new(), 1).await;
        drop(sender);
        let messages = drain(receiver).await;

        assert_eq!(
            texts(&messages),
            vec![(LogSource::Stdout, "avant".to_owned())]
        );
        assert!(matches!(
            messages.last(),
            Some(Message::StreamEnded { outcome: Err(reported), .. }) if *reported == error
        ));
    }

    #[tokio::test]
    async fn an_engine_that_refuses_the_stream_is_reported_as_an_end() {
        let error = HormosError::DaemonUnavailable {
            detail: "/var/run/docker.sock".into(),
        };
        let mock = Arc::new(MockRuntime::failing(error.clone()));
        let (sender, receiver) = mpsc::channel(16);

        pump_logs(&service(mock), &sender, "web", &LogOptions::new(), 3).await;
        drop(sender);

        assert!(matches!(
            drain(receiver).await.as_slice(),
            [Message::StreamEnded { generation: 3, outcome: Err(reported) }] if *reported == error
        ));
    }

    #[tokio::test]
    async fn events_are_published_one_by_one_then_closed() {
        let mock = Arc::new(MockRuntime::new());
        let (sender, receiver) = mpsc::channel(16);

        pump_events(&service(mock), &sender, 5).await;
        drop(sender);
        let messages = drain(receiver).await;

        let count = messages
            .iter()
            .filter(|message| matches!(message, Message::Event { generation: 5, .. }))
            .count();
        assert!(count > 0, "aucun événement publié");
        assert!(matches!(
            messages.last(),
            Some(Message::StreamEnded {
                generation: 5,
                outcome: Ok(())
            })
        ));
    }

    #[tokio::test]
    async fn an_endless_stream_stops_when_the_task_is_aborted() {
        let mock = Arc::new(MockRuntime::new().endless());
        let (sender, mut receiver) = mpsc::channel(1);
        let service = service(mock);
        let handle = tokio::spawn(async move {
            pump_events(&service, &sender, 1).await;
        });

        handle.abort();

        // Le canal se referme parce que la tâche — donc son émetteur — a bien
        // disparu. Attendre `join` bloquerait si l'interruption échouait.
        let closed = tokio::time::timeout(DEADLINE, receiver.recv())
            .await
            .expect("le flux interrompu n'a pas relâché son émetteur");
        assert!(closed.is_none());
    }
}

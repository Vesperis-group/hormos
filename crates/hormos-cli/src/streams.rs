//! Consommation des flux temps réel par la ligne de commande.
//!
//! Deux boucles, une par flux : les journaux et les événements. Elles partagent
//! trois propriétés :
//!
//! - **rien n'est accumulé** — chaque élément est écrit puis relâché ; aucun
//!   `collect`, aucune bufferisation de ligne ;
//! - **l'annulation est prioritaire** — la sélection est `biased`, la branche
//!   d'annulation est examinée avant celle du flux, de sorte qu'un flot continu
//!   ne peut pas retarder indéfiniment un `Ctrl+C` ;
//! - **l'annulation est un succès** — arrêter un suivi n'est pas une erreur, le
//!   code de sortie reste `0`. Une panne du moteur **pendant** le flux, elle,
//!   reste une erreur.
//!
//! # Assainissement conditionnel
//!
//! Un journal est écrit par le processus du conteneur : il peut contenir des
//! séquences d'échappement capables de piloter l'émulateur de terminal.
//!
//! - vers un **terminal**, la sortie est décodée et assainie
//!   ([`hormos_core::logs::LogDecoder`]) : les échappements sont neutralisés ;
//! - vers un **fichier ou un tube**, les octets sont recopiés **tels quels**.
//!   Il n'y a pas de terminal à protéger, et altérer les octets casserait toute
//!   chaîne de traitement (`hormos logs … | grep`, redirection, archivage).
//!
//! La décision est prise **par sortie** : `hormos logs web > fichier` assainit
//! encore `stderr` s'il est resté attaché au terminal.

use std::future::Future;
use std::io::{self, Write};

use hormos_core::error::{HormosError, Result};
use hormos_core::events::RuntimeEvent;
use hormos_core::logs::{LogChunk, LogDecoder, LogOptions, LogSource};
use hormos_core::service::ContainerService;

use crate::output;

/// Largeur de la colonne « type » du tableau d'événements.
const KIND_WIDTH: usize = 9;
/// Largeur de la colonne « action ».
const ACTION_WIDTH: usize = 16;
/// Longueur d'identifiant affichée, comme le fait Docker.
const ID_WIDTH: usize = 12;
/// Longueur maximale d'un nom affiché.
const NAME_WIDTH: usize = 32;

/// Sortie assainie ou brute, selon la destination.
struct Sink<W: Write> {
    writer: W,
    decoder: LogDecoder,
    sanitize: bool,
}

impl<W: Write> Sink<W> {
    fn new(writer: W, sanitize: bool) -> Self {
        Self {
            writer,
            decoder: LogDecoder::new(),
            sanitize,
        }
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if self.sanitize {
            self.writer.write_all(self.decoder.push(data).as_bytes())
        } else {
            self.writer.write_all(data)
        }
    }

    /// Vide l'état résiduel du décodeur puis la sortie elle-même.
    fn finish(&mut self) -> io::Result<()> {
        if self.sanitize {
            let tail = self.decoder.finish();
            if !tail.is_empty() {
                self.writer.write_all(tail.as_bytes())?;
            }
        }
        self.writer.flush()
    }
}

/// Écrit le journal d'un conteneur jusqu'à sa fin ou jusqu'à l'annulation.
///
/// `cancel` est le signal d'arrêt : `tokio::signal::ctrl_c` en production, un
/// futur contrôlé dans les tests.
///
/// # Errors
///
/// Propage l'erreur d'ouverture du flux, une panne survenue pendant le flux, ou
/// une erreur d'écriture — sauf la rupture de tube, qui est une fin normale.
pub(crate) async fn run_logs<O: Write, E: Write>(
    service: &ContainerService,
    reference: &str,
    options: &LogOptions,
    stdout: Sinkable<O>,
    stderr: Sinkable<E>,
    cancel: impl Future<Output = ()>,
) -> Result<()> {
    let mut stream = service.container_logs(reference, options)?;
    let mut out = Sink::new(stdout.writer, stdout.sanitize);
    let mut err = Sink::new(stderr.writer, stderr.sanitize);

    let mut failure = None;
    {
        let cancel = std::pin::pin!(cancel);
        let mut cancel = cancel;
        loop {
            let chunk = tokio::select! {
                biased;
                () = &mut cancel => break,
                item = stream.next() => item,
            };
            let Some(chunk) = chunk else { break };
            match chunk {
                Ok(chunk) => {
                    if let Err(error) = write_chunk(&chunk, &mut out, &mut err) {
                        // Un tube fermé en aval (`| head`) est une fin normale.
                        if error.kind() == io::ErrorKind::BrokenPipe {
                            break;
                        }
                        failure = Some(write_error(&error));
                        break;
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
    }

    // Les octets déjà reçus sont écrits même lorsque le flux se termine mal.
    let flushed = out.finish().and_then(|()| err.finish());
    match failure {
        Some(error) => Err(error),
        None => flushed.map_err(|error| write_error(&error)),
    }
}

/// Une sortie et la politique de rendu qui lui est associée.
pub(crate) struct Sinkable<W: Write> {
    /// Destination.
    pub writer: W,
    /// Assainir avant écriture (destination interactive).
    pub sanitize: bool,
}

impl<W: Write> std::fmt::Debug for Sinkable<W> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Sinkable")
            .field("sanitize", &self.sanitize)
            .finish_non_exhaustive()
    }
}

/// Route un fragment vers la sortie qui lui correspond.
///
/// `Console` rejoint `stdout` : un conteneur est soit attaché à un terminal —
/// et le moteur ne sépare alors pas les sorties — soit non. Les deux cas ne
/// coexistent jamais pour un même conteneur.
fn write_chunk<O: Write, E: Write>(
    chunk: &LogChunk,
    out: &mut Sink<O>,
    err: &mut Sink<E>,
) -> io::Result<()> {
    match chunk.source {
        LogSource::Stdout | LogSource::Console => out.write(&chunk.data),
        LogSource::Stderr => err.write(&chunk.data),
    }
}

/// Traduit une erreur d'écriture sans divulguer de chemin ni de descripteur.
fn write_error(error: &io::Error) -> HormosError {
    HormosError::runtime(format!("écriture de la sortie : {}", error.kind()))
}

/// Écrit les événements du moteur jusqu'à l'annulation.
///
/// En mode `json`, la sortie est du **NDJSON** : un objet complet par ligne,
/// consommable au fil de l'eau par un script, sans attendre la fin d'un tableau
/// qui n'arriverait jamais.
///
/// # Errors
///
/// Propage l'erreur d'abonnement, une panne survenue pendant le flux, ou une
/// erreur d'écriture — sauf la rupture de tube, qui est une fin normale.
pub(crate) async fn run_events<W: Write>(
    service: &ContainerService,
    json: bool,
    writer: &mut W,
    cancel: impl Future<Output = ()>,
) -> Result<()> {
    let mut stream = service.runtime_events()?;

    let mut failure = None;
    {
        let mut cancel = std::pin::pin!(cancel);
        loop {
            let item = tokio::select! {
                biased;
                () = &mut cancel => break,
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            match item {
                Ok(event) => {
                    let line = if json {
                        match output::to_json_line(&event) {
                            Ok(line) => line,
                            Err(error) => {
                                failure = Some(HormosError::runtime(format!(
                                    "sérialisation JSON : {error}"
                                )));
                                break;
                            }
                        }
                    } else {
                        render_event(&event)
                    };
                    // Chaque événement est vidé immédiatement : un flux suivi doit
                    // être lisible en direct, y compris derrière un tube.
                    if let Err(error) = writeln!(writer, "{line}").and_then(|()| writer.flush()) {
                        if error.kind() == io::ErrorKind::BrokenPipe {
                            break;
                        }
                        failure = Some(write_error(&error));
                        break;
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
    }

    failure.map_or_else(|| writer.flush().map_err(|error| write_error(&error)), Err)
}

/// Rend un événement en une ligne de tableau, entièrement assainie.
fn render_event(event: &RuntimeEvent) -> String {
    let kind = event.kind.as_str();
    let action = hormos_core::display::sanitize_truncated(&event.action, ACTION_WIDTH);
    let id = event.short_id(ID_WIDTH).unwrap_or_else(|| "-".to_owned());
    let id = hormos_core::display::sanitize_truncated(&id, ID_WIDTH);
    let name = event.actor_name.as_deref().map_or_else(
        || "-".to_owned(),
        |name| hormos_core::display::sanitize_truncated(name, NAME_WIDTH),
    );

    format!(
        "{:<20}  {kind:<KIND_WIDTH$}  {action:<ACTION_WIDTH$}  {id:<ID_WIDTH$}  {name}",
        event.formatted_time()
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::time::Duration;

    use hormos_core::error::{ErrorKind, HormosError};
    use hormos_core::events::{ResourceKind, RuntimeEvent};
    use hormos_core::logs::{LogChunk, LogOptions, LogSource, LogTail};
    use hormos_core::mock::{Call, MockRuntime};
    use hormos_core::service::ContainerService;

    use super::{Sinkable, run_events, run_logs};

    /// Jamais annulé.
    async fn never() {
        std::future::pending::<()>().await;
    }

    fn service(mock: Arc<MockRuntime>) -> ContainerService {
        ContainerService::new(mock)
    }

    async fn logs(mock: MockRuntime, sanitize: bool) -> (hormos_core::Result<()>, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_logs(
            &service(Arc::new(mock)),
            "web",
            &LogOptions::new(),
            Sinkable {
                writer: &mut out,
                sanitize,
            },
            Sinkable {
                writer: &mut err,
                sanitize,
            },
            never(),
        )
        .await;
        (
            outcome,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    #[tokio::test]
    async fn stdout_and_stderr_are_routed_separately() {
        let (outcome, out, err) = logs(MockRuntime::new(), false).await;
        assert!(outcome.is_ok());
        assert_eq!(out, "bonjour\n");
        assert_eq!(err, "attention\n");
    }

    #[tokio::test]
    async fn a_tty_container_writes_to_stdout() {
        let mock = MockRuntime::new().with_logs(vec![Ok(LogChunk::new(
            LogSource::Console,
            b"tty\n".to_vec(),
        ))]);
        let (outcome, out, err) = logs(mock, false).await;
        assert!(outcome.is_ok());
        assert_eq!(out, "tty\n");
        assert!(err.is_empty());
    }

    #[tokio::test]
    async fn a_pipe_receives_the_exact_bytes() {
        let hostile = vec![0x1b, b'[', b'2', b'K', 0xff, b'\n'];
        let mock = MockRuntime::new()
            .with_logs(vec![Ok(LogChunk::new(LogSource::Stdout, hostile.clone()))]);

        let mut out = Vec::new();
        let mut err = Vec::new();
        run_logs(
            &service(Arc::new(mock)),
            "web",
            &LogOptions::new(),
            Sinkable {
                writer: &mut out,
                sanitize: false,
            },
            Sinkable {
                writer: &mut err,
                sanitize: false,
            },
            never(),
        )
        .await
        .expect("le flux doit réussir");

        assert_eq!(out, hostile, "un tube doit recevoir les octets intacts");
    }

    #[tokio::test]
    async fn a_terminal_never_receives_an_escape_sequence() {
        let mock = MockRuntime::new().with_logs(vec![
            Ok(LogChunk::new(
                LogSource::Stdout,
                b"\x1b[2Ktrompeur\n".to_vec(),
            )),
            Ok(LogChunk::new(
                LogSource::Stderr,
                vec![0x1b, b']', b'0', 0x07],
            )),
        ]);
        let (outcome, out, err) = logs(mock, true).await;

        assert!(outcome.is_ok());
        assert_eq!(out, "\u{fffd}[2Ktrompeur\n");
        assert_eq!(err, "\u{fffd}]0\u{fffd}");
        assert!(!out.contains('\u{1b}'), "un échappement a atteint l'écran");
        assert!(!err.contains('\u{1b}'), "un échappement a atteint l'écran");
    }

    #[tokio::test]
    async fn a_character_split_across_chunks_is_reassembled() {
        let smiley = "🙂".as_bytes();
        let mock = MockRuntime::new().with_logs(vec![
            Ok(LogChunk::new(LogSource::Stdout, smiley[..2].to_vec())),
            Ok(LogChunk::new(LogSource::Stdout, smiley[2..].to_vec())),
        ]);
        let (outcome, out, _) = logs(mock, true).await;
        assert!(outcome.is_ok());
        assert_eq!(out, "🙂");
    }

    #[tokio::test]
    async fn an_error_mid_stream_keeps_what_was_already_written() {
        let mock = MockRuntime::new().with_logs(vec![
            Ok(LogChunk::new(LogSource::Stdout, b"avant\n".to_vec())),
            Err(HormosError::runtime("le démon a coupé")),
            Ok(LogChunk::new(LogSource::Stdout, b"jamais\n".to_vec())),
        ]);
        let (outcome, out, _) = logs(mock, false).await;

        assert_eq!(
            outcome.map_err(|error| error.kind()),
            Err(ErrorKind::RuntimeError)
        );
        assert_eq!(out, "avant\n", "la sortie déjà reçue doit être conservée");
    }

    #[tokio::test]
    async fn opening_failure_is_reported_before_anything_is_written() {
        let mock = MockRuntime::failing(HormosError::ContainerNotFound {
            reference: "web".into(),
        });
        let (outcome, out, err) = logs(mock, false).await;

        assert_eq!(
            outcome.map_err(|error| error.kind()),
            Err(ErrorKind::ContainerNotFound)
        );
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[tokio::test]
    async fn cancelling_a_follow_is_a_success() {
        let mock = Arc::new(MockRuntime::new().endless());
        let mut out = Vec::new();
        let mut err = Vec::new();

        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_logs(
                &service(Arc::clone(&mock)),
                "web",
                &LogOptions::new().follow(true),
                Sinkable {
                    writer: &mut out,
                    sanitize: true,
                },
                Sinkable {
                    writer: &mut err,
                    sanitize: true,
                },
                tokio::time::sleep(Duration::from_millis(10)),
            ),
        )
        .await
        .expect("l'annulation doit être prise en compte rapidement");

        assert!(outcome.is_ok(), "annuler un suivi n'est pas une erreur");
        assert_eq!(
            mock.calls(),
            vec![Call::Logs("web".into(), LogOptions::new().follow(true))]
        );
    }

    #[tokio::test]
    async fn cancellation_wins_over_an_endless_flood() {
        // Le flux ne se tarit jamais : seule la priorité `biased` garantit que
        // l'annulation est examinée avant lui.
        let flood: Vec<hormos_core::Result<LogChunk>> = (0..50_000)
            .map(|_| Ok(LogChunk::new(LogSource::Stdout, b"x\n".to_vec())))
            .collect();
        let mock = Arc::new(MockRuntime::new().with_logs(flood));

        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_logs(
                &service(mock),
                "web",
                &LogOptions::new().follow(true),
                Sinkable {
                    writer: &mut out,
                    sanitize: true,
                },
                Sinkable {
                    writer: &mut err,
                    sanitize: true,
                },
                std::future::ready(()),
            ),
        )
        .await
        .expect("l'annulation doit trancher immédiatement");

        assert!(outcome.is_ok());
        assert!(
            out.is_empty(),
            "l'annulation devait précéder toute écriture"
        );
    }

    #[tokio::test]
    async fn tail_and_timestamps_reach_the_runtime() {
        let mock = Arc::new(MockRuntime::new());
        let options = LogOptions::new().tail(LogTail::Lines(25)).timestamps(true);
        let mut out = Vec::new();
        let mut err = Vec::new();

        run_logs(
            &service(Arc::clone(&mock)),
            "web",
            &options,
            Sinkable {
                writer: &mut out,
                sanitize: false,
            },
            Sinkable {
                writer: &mut err,
                sanitize: false,
            },
            never(),
        )
        .await
        .expect("le flux doit réussir");

        assert_eq!(mock.calls(), vec![Call::Logs("web".into(), options)]);
    }

    fn sample_event(action: &str, name: Option<&str>) -> RuntimeEvent {
        RuntimeEvent {
            timestamp: Some(1_700_000_000),
            kind: ResourceKind::Container,
            action: action.to_owned(),
            actor_id: Some("0123456789abcdef0123".to_owned()),
            actor_name: name.map(str::to_owned),
        }
    }

    async fn events(mock: MockRuntime, json: bool) -> (hormos_core::Result<()>, String) {
        let mut out = Vec::new();
        let outcome = run_events(&service(Arc::new(mock)), json, &mut out, never()).await;
        (outcome, String::from_utf8_lossy(&out).into_owned())
    }

    #[tokio::test]
    async fn events_are_rendered_as_a_sanitized_table() {
        let mock =
            MockRuntime::new().with_events(vec![Ok(sample_event("sta\u{1b}[2Krt", Some("we\nb")))]);
        let (outcome, out) = events(mock, false).await;

        assert!(outcome.is_ok());
        assert!(
            !out.contains('\u{1b}'),
            "échappement non neutralisé : {out}"
        );
        assert!(out.contains("2023-11-14T22:13:20Z"));
        assert!(out.contains("container"));
        assert!(out.contains("0123456789ab"), "identifiant tronqué à 12");
        assert!(out.contains("we\u{fffd}b"));
        assert_eq!(out.lines().count(), 1);
    }

    #[tokio::test]
    async fn missing_fields_are_displayed_as_dashes() {
        let mock = MockRuntime::new().with_events(vec![Ok(RuntimeEvent {
            timestamp: None,
            kind: ResourceKind::Other,
            action: "?".to_owned(),
            actor_id: None,
            actor_name: None,
        })]);
        let (outcome, out) = events(mock, false).await;

        assert!(outcome.is_ok());
        assert!(out.starts_with('-'), "horodatage manquant : {out}");
        assert!(out.contains("other"));
    }

    #[tokio::test]
    async fn json_events_are_one_object_per_line() {
        let mock = MockRuntime::new().with_events(vec![
            Ok(sample_event("start", Some("web"))),
            Ok(sample_event("die", None)),
        ]);
        let (outcome, out) = events(mock, true).await;

        assert!(outcome.is_ok());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("chaque ligne est un objet JSON complet");
            assert_eq!(parsed["kind"], "container");
        }
    }

    #[tokio::test]
    async fn json_escapes_control_characters() {
        let mock =
            MockRuntime::new().with_events(vec![Ok(sample_event("sta\u{1b}rt", Some("web")))]);
        let (outcome, out) = events(mock, true).await;

        assert!(outcome.is_ok());
        assert!(!out.contains('\u{1b}'), "échappement brut en JSON : {out}");
        assert!(out.contains("\\u001b"));
    }

    #[tokio::test]
    async fn an_event_stream_failure_is_reported() {
        let mock = MockRuntime::new().with_events(vec![
            Ok(sample_event("start", Some("web"))),
            Err(HormosError::runtime("le démon a coupé")),
        ]);
        let (outcome, out) = events(mock, false).await;

        assert_eq!(
            outcome.map_err(|error| error.kind()),
            Err(ErrorKind::RuntimeError)
        );
        assert_eq!(out.lines().count(), 1);
    }

    #[tokio::test]
    async fn cancelling_an_event_stream_is_a_success() {
        let mock = Arc::new(MockRuntime::new().endless());
        let mut out = Vec::new();

        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_events(
                &service(mock),
                false,
                &mut out,
                tokio::time::sleep(Duration::from_millis(10)),
            ),
        )
        .await
        .expect("l'annulation doit être prise en compte rapidement");

        assert!(outcome.is_ok());
        assert!(out.is_empty());
    }
}

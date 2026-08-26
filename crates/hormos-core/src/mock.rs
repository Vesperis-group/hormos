//! Faux moteur déterministe, réservé aux tests du cœur.
//!
//! Ce n'est **pas** une réimplémentation de Docker : il enregistre les appels
//! reçus et renvoie des valeurs figées, afin de vérifier que le service valide
//! les entrées, transmet les bons arguments et propage les erreurs.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::{ContainerDetails, ContainerState, ContainerSummary, SystemInfo};
use crate::error::{HormosError, Result};
use crate::events::{ResourceKind, RuntimeEvent};
use crate::logs::{LogChunk, LogOptions, LogSource};
use crate::reference::ContainerRef;
use crate::runtime::ContainerRuntime;
use crate::stream::RuntimeStream;

/// Appel reçu par le faux moteur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// `system_info`.
    SystemInfo,
    /// `list_containers(all)`.
    List(bool),
    /// `inspect_container(reference)`.
    Inspect(String),
    /// `start_container(reference)`.
    Start(String),
    /// `stop_container(reference)`.
    Stop(String),
    /// `restart_container(reference)`.
    Restart(String),
    /// `container_logs(reference, options)`.
    Logs(String, LogOptions),
    /// `runtime_events()`.
    Events,
}

/// Faux moteur enregistrant les appels.
#[derive(Debug)]
pub struct MockRuntime {
    calls: Mutex<Vec<Call>>,
    failure: Option<HormosError>,
    logs: Mutex<Option<Vec<Result<LogChunk>>>>,
    events: Mutex<Option<Vec<Result<RuntimeEvent>>>>,
    endless: bool,
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRuntime {
    /// Faux moteur qui réussit toujours.
    pub const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
            logs: Mutex::new(None),
            events: Mutex::new(None),
            endless: false,
        }
    }

    /// Faux moteur qui échoue toujours avec l'erreur fournie.
    pub const fn failing(error: HormosError) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(error),
            logs: Mutex::new(None),
            events: Mutex::new(None),
            endless: false,
        }
    }

    /// Impose la séquence exacte que produira le flux de journal.
    #[must_use]
    pub fn with_logs(self, items: Vec<Result<LogChunk>>) -> Self {
        if let Ok(mut logs) = self.logs.lock() {
            *logs = Some(items);
        }
        self
    }

    /// Impose la séquence exacte que produira le flux d'événements.
    #[must_use]
    pub fn with_events(self, items: Vec<Result<RuntimeEvent>>) -> Self {
        if let Ok(mut events) = self.events.lock() {
            *events = Some(items);
        }
        self
    }

    /// Rend les flux muets et sans fin, pour éprouver l'annulation.
    #[must_use]
    pub const fn endless(mut self) -> Self {
        self.endless = true;
        self
    }

    /// Appels enregistrés, dans l'ordre.
    pub fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    fn record(&self, call: Call) -> Result<()> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
        self.failure.clone().map_or(Ok(()), Err)
    }

    fn summary(name: &str, state: ContainerState) -> ContainerSummary {
        ContainerSummary {
            id: format!("id-{name}"),
            name: name.to_owned(),
            image: "example:1".to_owned(),
            status: state.as_str().to_owned(),
            state,
            created: Some(1_700_000_000),
        }
    }

    fn default_logs() -> Vec<Result<LogChunk>> {
        vec![
            Ok(LogChunk::new(LogSource::Stdout, b"bonjour\n".to_vec())),
            Ok(LogChunk::new(LogSource::Stderr, b"attention\n".to_vec())),
        ]
    }

    fn default_events() -> Vec<Result<RuntimeEvent>> {
        vec![Ok(RuntimeEvent {
            timestamp: Some(1_700_000_000),
            kind: ResourceKind::Container,
            action: "start".to_owned(),
            actor_id: Some("id-web".to_owned()),
            actor_name: Some("web".to_owned()),
        })]
    }
}

#[async_trait]
impl ContainerRuntime for MockRuntime {
    async fn system_info(&self) -> Result<SystemInfo> {
        self.record(Call::SystemInfo)?;
        Ok(SystemInfo {
            server_version: Some("0.0.0-mock".to_owned()),
            api_version: Some("1.51".to_owned()),
            os: Some("linux".to_owned()),
            architecture: Some("x86_64".to_owned()),
            containers_total: Some(2),
            containers_running: Some(1),
            endpoint: "mock".to_owned(),
        })
    }

    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerSummary>> {
        self.record(Call::List(all))?;
        let mut containers = vec![Self::summary("web", ContainerState::Running)];
        if all {
            containers.push(Self::summary("worker", ContainerState::Exited));
        }
        Ok(containers)
    }

    async fn inspect_container(&self, reference: &ContainerRef) -> Result<ContainerDetails> {
        self.record(Call::Inspect(reference.as_str().to_owned()))?;
        Ok(ContainerDetails {
            id: format!("id-{reference}"),
            name: reference.as_str().to_owned(),
            image: "example:1".to_owned(),
            state: ContainerState::Running,
            status: Some("Up 2 hours".to_owned()),
            created: Some("2026-01-01T00:00:00Z".to_owned()),
            hostname: Some("mock-host".to_owned()),
            restart_count: Some(0),
        })
    }

    async fn start_container(&self, reference: &ContainerRef) -> Result<()> {
        self.record(Call::Start(reference.as_str().to_owned()))
    }

    async fn stop_container(&self, reference: &ContainerRef) -> Result<()> {
        self.record(Call::Stop(reference.as_str().to_owned()))
    }

    async fn restart_container(&self, reference: &ContainerRef) -> Result<()> {
        self.record(Call::Restart(reference.as_str().to_owned()))
    }

    fn container_logs(
        &self,
        reference: &ContainerRef,
        options: &LogOptions,
    ) -> Result<RuntimeStream<LogChunk>> {
        self.record(Call::Logs(reference.as_str().to_owned(), *options))?;
        if self.endless {
            return Ok(RuntimeStream::never());
        }
        let scripted = self.logs.lock().ok().and_then(|logs| logs.clone());
        Ok(RuntimeStream::from_items(
            scripted.unwrap_or_else(Self::default_logs),
        ))
    }

    fn runtime_events(&self) -> Result<RuntimeStream<RuntimeEvent>> {
        self.record(Call::Events)?;
        if self.endless {
            return Ok(RuntimeStream::never());
        }
        let scripted = self.events.lock().ok().and_then(|events| events.clone());
        Ok(RuntimeStream::from_items(
            scripted.unwrap_or_else(Self::default_events),
        ))
    }
}

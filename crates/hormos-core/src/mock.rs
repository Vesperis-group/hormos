//! Faux moteur déterministe, réservé aux tests du cœur.
//!
//! Ce n'est **pas** une réimplémentation de Docker : il enregistre les appels
//! reçus et renvoie des valeurs figées, afin de vérifier que le service valide
//! les entrées, transmet les bons arguments et propage les erreurs.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::{ContainerDetails, ContainerState, ContainerSummary, SystemInfo};
use crate::error::{HormosError, Result};
use crate::reference::ContainerRef;
use crate::runtime::ContainerRuntime;

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
}

/// Faux moteur enregistrant les appels.
#[derive(Debug)]
pub struct MockRuntime {
    calls: Mutex<Vec<Call>>,
    failure: Option<HormosError>,
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
        }
    }

    /// Faux moteur qui échoue toujours avec l'erreur fournie.
    pub const fn failing(error: HormosError) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(error),
        }
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
}

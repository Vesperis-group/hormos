//! Implémentation du trait [`ContainerRuntime`] au-dessus de Bollard.
//!
//! Cet adaptateur est le **seul** endroit du dépôt qui parle à Docker. Il
//! applique trois règles :
//!
//! 1. **socket local uniquement** — le point de terminaison est résolu et validé
//!    par [`LocalEndpoint`], et le client est construit avec
//!    `connect_with_unix`, disponible seulement grâce à la fonctionnalité
//!    `pipe` de Bollard ; les transports TCP/HTTP ne sont pas compilés ;
//! 2. **aucune opération sans délai** — chaque appel est encadré par un
//!    [`tokio::time::timeout`] issu de [`crate::timeouts`] ;
//! 3. **rien ne fuit** — les erreurs Bollard sont traduites par
//!    [`crate::error::map_error`] et les réponses par [`crate::mapping`].
//!
//! Aucune commande n'est construite par concaténation de chaîne : les
//! références de conteneurs sont passées telles quelles à Bollard, après
//! validation par [`ContainerRef`], et aucun shell n'est invoqué.

use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{
    InspectContainerOptionsBuilder, ListContainersOptionsBuilder, RestartContainerOptionsBuilder,
    StartContainerOptionsBuilder, StopContainerOptionsBuilder,
};
use hormos_core::domain::{ContainerDetails, ContainerSummary, SystemInfo};
use hormos_core::error::{HormosError, Result};
use hormos_core::reference::ContainerRef;
use hormos_core::runtime::ContainerRuntime;

use crate::endpoint::LocalEndpoint;
use crate::error::map_error;
use crate::mapping;
use crate::timeouts;

/// Moteur Docker joint par socket Unix local.
pub struct DockerRuntime {
    docker: Docker,
    endpoint: String,
    api_version: String,
}

impl std::fmt::Debug for DockerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Docker` n'est pas `Debug` ; on n'expose que ce qui est utile et sûr.
        f.debug_struct("DockerRuntime")
            .field("endpoint", &self.endpoint)
            .field("api_version", &self.api_version)
            .finish()
    }
}

impl DockerRuntime {
    /// Se connecte au moteur local résolu depuis l'environnement.
    ///
    /// # Errors
    ///
    /// Voir [`DockerRuntime::connect_to`].
    pub async fn connect() -> Result<Self> {
        Self::connect_to(&LocalEndpoint::from_env()?).await
    }

    /// Se connecte à un point de terminaison local déjà validé.
    ///
    /// La connexion négocie la version de l'API puis vérifie la disponibilité du
    /// moteur (`ping`) : `hormos` échoue immédiatement et clairement plutôt que
    /// de laisser la première commande utile partir en erreur obscure.
    ///
    /// # Errors
    ///
    /// [`HormosError::DaemonUnavailable`] si le socket est absent ou refusé,
    /// [`HormosError::PermissionDenied`] si l'utilisateur n'a pas accès au
    /// socket, [`HormosError::Timeout`] si le moteur ne répond pas.
    pub async fn connect_to(endpoint: &LocalEndpoint) -> Result<Self> {
        let path = endpoint.as_str()?.to_owned();

        let docker = Docker::connect_with_unix(
            &path,
            timeouts::CLIENT_SECONDS,
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(|error| map_error(&error, &path, None))?;

        let docker = guard("connexion Docker", timeouts::NEGOTIATE_SECONDS, {
            let path = path.clone();
            async move {
                docker
                    .negotiate_version()
                    .await
                    .map_err(|error| map_error(&error, &path, None))
            }
        })
        .await?;

        guard("ping Docker", timeouts::PING_SECONDS, {
            let docker = docker.clone();
            let path = path.clone();
            async move {
                docker
                    .ping()
                    .await
                    .map(|_| ())
                    .map_err(|error| map_error(&error, &path, None))
            }
        })
        .await?;

        let version = docker.client_version();
        let api_version = format!("{}.{}", version.major_version, version.minor_version);

        Ok(Self {
            docker,
            endpoint: path,
            api_version,
        })
    }

    /// Point de terminaison réellement utilisé.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Version de l'API négociée avec le moteur.
    #[must_use]
    pub fn api_version(&self) -> &str {
        &self.api_version
    }

    /// Encadre un appel Bollard par un délai et traduit son erreur.
    async fn call<T, F>(&self, operation: &'static str, seconds: u64, future: F) -> Result<T>
    where
        F: Future<Output = std::result::Result<T, bollard::errors::Error>>,
    {
        guard(operation, seconds, async {
            future
                .await
                .map_err(|error| map_error(&error, &self.endpoint, None))
        })
        .await
    }

    /// Variante de [`Self::call`] pour les opérations portant sur un conteneur.
    async fn call_on<T, F>(
        &self,
        operation: &'static str,
        seconds: u64,
        reference: &ContainerRef,
        future: F,
    ) -> Result<T>
    where
        F: Future<Output = std::result::Result<T, bollard::errors::Error>>,
    {
        guard(operation, seconds, async {
            future
                .await
                .map_err(|error| map_error(&error, &self.endpoint, Some(reference.as_str())))
        })
        .await
    }
}

/// Applique un délai maximal à une opération.
async fn guard<T, F>(operation: &'static str, seconds: u64, future: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::time::timeout(Duration::from_secs(seconds), future)
        .await
        .unwrap_or_else(|_| Err(HormosError::Timeout { operation, seconds }))
}

#[async_trait]
impl ContainerRuntime for DockerRuntime {
    async fn system_info(&self) -> Result<SystemInfo> {
        let info = self
            .call("info", timeouts::INFO_SECONDS, self.docker.info())
            .await?;
        Ok(mapping::to_system_info(
            info,
            self.api_version.clone(),
            self.endpoint.clone(),
        ))
    }

    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerSummary>> {
        let options = ListContainersOptionsBuilder::new().all(all).build();
        let containers = self
            .call(
                "list",
                timeouts::LIST_SECONDS,
                self.docker.list_containers(Some(options)),
            )
            .await?;
        Ok(containers.into_iter().map(mapping::to_summary).collect())
    }

    async fn inspect_container(&self, reference: &ContainerRef) -> Result<ContainerDetails> {
        let options = InspectContainerOptionsBuilder::new().size(false).build();
        let response = self
            .call_on(
                "inspect",
                timeouts::INSPECT_SECONDS,
                reference,
                self.docker
                    .inspect_container(reference.as_str(), Some(options)),
            )
            .await?;
        Ok(mapping::to_details(response))
    }

    async fn start_container(&self, reference: &ContainerRef) -> Result<()> {
        // Un conteneur déjà démarré répond 304, que Bollard traite en succès :
        // l'opération est donc idempotente sans traitement particulier.
        let options = StartContainerOptionsBuilder::new().build();
        self.call_on(
            "start",
            timeouts::START_SECONDS,
            reference,
            self.docker
                .start_container(reference.as_str(), Some(options)),
        )
        .await
    }

    async fn stop_container(&self, reference: &ContainerRef) -> Result<()> {
        // Idem : un conteneur déjà arrêté répond 304.
        let options = StopContainerOptionsBuilder::new().build();
        self.call_on(
            "stop",
            timeouts::STOP_SECONDS,
            reference,
            self.docker
                .stop_container(reference.as_str(), Some(options)),
        )
        .await
    }

    async fn restart_container(&self, reference: &ContainerRef) -> Result<()> {
        let options = RestartContainerOptionsBuilder::new().build();
        self.call_on(
            "restart",
            timeouts::RESTART_SECONDS,
            reference,
            self.docker
                .restart_container(reference.as_str(), Some(options)),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hormos_core::error::{ErrorKind, HormosError};

    use super::guard;

    #[tokio::test]
    async fn guard_returns_the_value_when_fast_enough() {
        let value = guard("test", 5, async { Ok(42_u8) }).await;
        assert_eq!(value, Ok(42));
    }

    #[tokio::test]
    async fn guard_converts_a_slow_call_into_a_timeout() {
        tokio::time::pause();
        let slow = guard("lent", 1, async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok(())
        });
        let outcome = slow.await;
        assert_eq!(
            outcome,
            Err(HormosError::Timeout {
                operation: "lent",
                seconds: 1
            })
        );
    }

    #[tokio::test]
    async fn guard_preserves_the_inner_error() {
        let outcome: Result<(), HormosError> =
            guard("test", 5, async { Err(HormosError::runtime("boom")) }).await;
        assert_eq!(outcome.map_err(|e| e.kind()), Err(ErrorKind::RuntimeError));
    }
}

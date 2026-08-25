//! Cas d'usage d'Hormos.
//!
//! [`ContainerService`] est la **seule** porte d'entrée des interfaces : elles ne
//! parlent jamais directement à un [`ContainerRuntime`]. Le service valide les
//! entrées puis délègue. Une seule couche : ni « repository », ni « manager »,
//! ni « handler ».

use std::sync::Arc;

use crate::domain::{ContainerDetails, ContainerSummary, SystemInfo};
use crate::error::Result;
use crate::reference::ContainerRef;
use crate::runtime::ContainerRuntime;

/// Service exposant les cas d'usage conteneurs de cette phase.
#[derive(Debug, Clone)]
pub struct ContainerService {
    runtime: Arc<dyn ContainerRuntime>,
}

impl ContainerService {
    /// Construit le service au-dessus d'un moteur quelconque.
    #[must_use]
    pub fn new(runtime: Arc<dyn ContainerRuntime>) -> Self {
        Self { runtime }
    }

    /// Informations sur le moteur.
    ///
    /// # Errors
    ///
    /// Propage l'erreur du moteur (indisponible, permission, délai dépassé…).
    pub async fn system_info(&self) -> Result<SystemInfo> {
        self.runtime.system_info().await
    }

    /// Liste les conteneurs en cours d'exécution, ou tous si `all`.
    ///
    /// # Errors
    ///
    /// Propage l'erreur du moteur.
    pub async fn list_containers(&self, all: bool) -> Result<Vec<ContainerSummary>> {
        self.runtime.list_containers(all).await
    }

    /// Détail minimal d'un conteneur.
    ///
    /// # Errors
    ///
    /// [`crate::HormosError::InvalidInput`] si la référence est invalide — dans
    /// ce cas le moteur n'est **pas** appelé. Sinon, propage l'erreur du moteur.
    pub async fn inspect_container(&self, reference: &str) -> Result<ContainerDetails> {
        let reference = ContainerRef::new(reference)?;
        self.runtime.inspect_container(&reference).await
    }

    /// Démarre un conteneur et retourne la référence validée.
    ///
    /// # Errors
    ///
    /// [`crate::HormosError::InvalidInput`] si la référence est invalide — dans
    /// ce cas le moteur n'est **pas** appelé. Sinon, propage l'erreur du moteur.
    pub async fn start_container(&self, reference: &str) -> Result<ContainerRef> {
        let reference = ContainerRef::new(reference)?;
        self.runtime.start_container(&reference).await?;
        Ok(reference)
    }

    /// Arrête un conteneur et retourne la référence validée.
    ///
    /// # Errors
    ///
    /// [`crate::HormosError::InvalidInput`] si la référence est invalide — dans
    /// ce cas le moteur n'est **pas** appelé. Sinon, propage l'erreur du moteur.
    pub async fn stop_container(&self, reference: &str) -> Result<ContainerRef> {
        let reference = ContainerRef::new(reference)?;
        self.runtime.stop_container(&reference).await?;
        Ok(reference)
    }

    /// Redémarre un conteneur et retourne la référence validée.
    ///
    /// # Errors
    ///
    /// [`crate::HormosError::InvalidInput`] si la référence est invalide — dans
    /// ce cas le moteur n'est **pas** appelé. Sinon, propage l'erreur du moteur.
    pub async fn restart_container(&self, reference: &str) -> Result<ContainerRef> {
        let reference = ContainerRef::new(reference)?;
        self.runtime.restart_container(&reference).await?;
        Ok(reference)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ContainerService;
    use crate::domain::ContainerState;
    use crate::error::{ErrorKind, HormosError};
    use crate::mock::{Call, MockRuntime};

    fn service(mock: Arc<MockRuntime>) -> ContainerService {
        ContainerService::new(mock)
    }

    #[tokio::test]
    async fn system_info_is_forwarded() {
        let mock = Arc::new(MockRuntime::new());
        let info = service(Arc::clone(&mock))
            .system_info()
            .await
            .map(|i| i.endpoint);
        assert_eq!(info.as_deref(), Ok("mock"));
        assert_eq!(mock.calls(), vec![Call::SystemInfo]);
    }

    #[tokio::test]
    async fn list_running_and_all_are_distinct() {
        let mock = Arc::new(MockRuntime::new());
        let service = service(Arc::clone(&mock));

        let running = service.list_containers(false).await.map(|c| c.len());
        assert_eq!(running, Ok(1));

        let all = service.list_containers(true).await.map(|c| c.len());
        assert_eq!(all, Ok(2));

        assert_eq!(mock.calls(), vec![Call::List(false), Call::List(true)]);
    }

    #[tokio::test]
    async fn inspect_returns_details() {
        let mock = Arc::new(MockRuntime::new());
        let details = service(Arc::clone(&mock)).inspect_container("web").await;
        let state = details.map(|d| d.state);
        assert_eq!(state, Ok(ContainerState::Running));
        assert_eq!(mock.calls(), vec![Call::Inspect("web".into())]);
    }

    #[tokio::test]
    async fn lifecycle_actions_are_forwarded() {
        let mock = Arc::new(MockRuntime::new());
        let service = service(Arc::clone(&mock));

        assert!(service.start_container("web").await.is_ok());
        assert!(service.stop_container("web").await.is_ok());
        assert!(service.restart_container("web").await.is_ok());

        assert_eq!(
            mock.calls(),
            vec![
                Call::Start("web".into()),
                Call::Stop("web".into()),
                Call::Restart("web".into()),
            ]
        );
    }

    #[tokio::test]
    async fn invalid_reference_never_reaches_the_runtime() {
        for value in ["", "  ", "ngi\u{0}nx", "../etc", "nginx/json"] {
            let mock = Arc::new(MockRuntime::new());
            let service = service(Arc::clone(&mock));

            let kind = service
                .inspect_container(value)
                .await
                .map(|_| ())
                .map_err(|e| e.kind());
            assert_eq!(
                kind,
                Err(ErrorKind::InvalidInput),
                "accepté à tort : {value}"
            );

            assert!(service.start_container(value).await.is_err());
            assert!(service.stop_container(value).await.is_err());
            assert!(service.restart_container(value).await.is_err());

            assert!(
                mock.calls().is_empty(),
                "le moteur a été appelé malgré une entrée invalide : {value}"
            );
        }
    }

    #[tokio::test]
    async fn runtime_errors_are_propagated_unchanged() {
        let expected = HormosError::ContainerNotFound {
            reference: "web".into(),
        };
        let mock = Arc::new(MockRuntime::failing(expected.clone()));
        let service = service(Arc::clone(&mock));

        assert_eq!(
            service.system_info().await.map(|_| ()),
            Err(expected.clone())
        );
        assert_eq!(
            service.list_containers(true).await.map(|_| ()),
            Err(expected.clone())
        );
        assert_eq!(
            service.inspect_container("web").await.map(|_| ()),
            Err(expected.clone())
        );
        assert_eq!(
            service.start_container("web").await.map(|_| ()),
            Err(expected.clone())
        );
        assert_eq!(
            service.stop_container("web").await.map(|_| ()),
            Err(expected.clone())
        );
        assert_eq!(
            service.restart_container("web").await.map(|_| ()),
            Err(expected)
        );
    }
}

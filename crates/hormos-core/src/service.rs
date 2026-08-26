//! Cas d'usage d'Hormos.
//!
//! [`ContainerService`] est la **seule** porte d'entrée des interfaces : elles ne
//! parlent jamais directement à un [`ContainerRuntime`]. Le service valide les
//! entrées puis délègue. Une seule couche : ni « repository », ni « manager »,
//! ni « handler ».

use std::sync::Arc;

use crate::domain::{ContainerDetails, ContainerSummary, SystemInfo};
use crate::error::Result;
use crate::events::RuntimeEvent;
use crate::logs::{LogChunk, LogOptions};
use crate::reference::ContainerRef;
use crate::runtime::ContainerRuntime;
use crate::stream::RuntimeStream;

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

    /// Ouvre le journal d'un conteneur.
    ///
    /// La référence est validée **avant** d'atteindre le moteur, comme pour toute
    /// autre opération : une entrée hostile n'ouvre jamais de connexion.
    ///
    /// # Errors
    ///
    /// [`crate::HormosError::InvalidInput`] si la référence est invalide — dans
    /// ce cas le moteur n'est **pas** appelé. Sinon, propage l'erreur du moteur.
    pub fn container_logs(
        &self,
        reference: &str,
        options: &LogOptions,
    ) -> Result<RuntimeStream<LogChunk>> {
        let reference = ContainerRef::new(reference)?;
        self.runtime.container_logs(&reference, options)
    }

    /// S'abonne au flux d'événements du moteur.
    ///
    /// # Errors
    ///
    /// Propage l'erreur du moteur.
    pub fn runtime_events(&self) -> Result<RuntimeStream<RuntimeEvent>> {
        self.runtime.runtime_events()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::sync::Arc;
    use std::time::Duration;

    use super::ContainerService;
    use crate::domain::ContainerState;
    use crate::error::{ErrorKind, HormosError};
    use crate::logs::{LogChunk, LogOptions, LogSource, LogTail};
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

    #[tokio::test]
    async fn logs_forward_the_reference_and_the_options() {
        let mock = Arc::new(MockRuntime::new());
        let options = LogOptions::new()
            .follow(true)
            .tail(LogTail::Lines(10))
            .timestamps(true);

        let mut stream = service(Arc::clone(&mock))
            .container_logs("web", &options)
            .expect("le flux doit s'ouvrir");

        assert_eq!(
            stream.next().await,
            Some(Ok(LogChunk::new(LogSource::Stdout, b"bonjour\n".to_vec())))
        );
        assert_eq!(
            stream.next().await,
            Some(Ok(LogChunk::new(
                LogSource::Stderr,
                b"attention\n".to_vec()
            )))
        );
        assert_eq!(stream.next().await, None);
        assert_eq!(mock.calls(), vec![Call::Logs("web".into(), options)]);
    }

    #[tokio::test]
    async fn logs_reject_an_invalid_reference_before_the_runtime() {
        for value in ["", "  ", "ngi\u{0}nx", "../etc", "nginx/json"] {
            let mock = Arc::new(MockRuntime::new());
            let outcome = service(Arc::clone(&mock))
                .container_logs(value, &LogOptions::new())
                .map(|_| ())
                .map_err(|error| error.kind());
            assert_eq!(
                outcome,
                Err(ErrorKind::InvalidInput),
                "accepté à tort : {value}"
            );
            assert!(
                mock.calls().is_empty(),
                "le moteur a été appelé malgré une entrée invalide : {value}"
            );
        }
    }

    #[tokio::test]
    async fn an_error_raised_mid_stream_is_delivered_as_an_item() {
        let boom = HormosError::runtime("le démon a coupé");
        let mock = Arc::new(MockRuntime::new().with_logs(vec![
            Ok(LogChunk::new(LogSource::Stdout, b"a".to_vec())),
            Err(boom.clone()),
        ]));

        let mut stream = service(mock)
            .container_logs("web", &LogOptions::new())
            .expect("le flux doit s'ouvrir");

        assert!(matches!(stream.next().await, Some(Ok(_))));
        assert_eq!(stream.next().await, Some(Err(boom)));
        assert_eq!(stream.next().await, None);
    }

    #[tokio::test]
    async fn opening_a_stream_can_fail_immediately() {
        let expected = HormosError::ContainerNotFound {
            reference: "web".into(),
        };
        let mock = Arc::new(MockRuntime::failing(expected.clone()));
        let service = service(mock);

        assert_eq!(
            service
                .container_logs("web", &LogOptions::new())
                .map(|_| ()),
            Err(expected.clone())
        );
        assert_eq!(service.runtime_events().map(|_| ()), Err(expected));
    }

    #[tokio::test]
    async fn events_are_forwarded() {
        let mock = Arc::new(MockRuntime::new());
        let mut stream = service(Arc::clone(&mock))
            .runtime_events()
            .expect("l'abonnement doit s'ouvrir");

        let event = stream.next().await.and_then(Result::ok);
        assert_eq!(event.map(|e| e.action), Some("start".to_owned()));
        assert_eq!(stream.next().await, None);
        assert_eq!(mock.calls(), vec![Call::Events]);
    }

    #[tokio::test]
    async fn dropping_a_stream_is_enough_to_cancel_it() {
        let mock = Arc::new(MockRuntime::new().endless());
        let mut stream = service(mock)
            .runtime_events()
            .expect("l'abonnement doit s'ouvrir");

        let outcome = tokio::time::timeout(Duration::from_millis(20), stream.next()).await;
        assert!(outcome.is_err(), "un flux sans fin ne produit rien");
        drop(stream);
    }
}

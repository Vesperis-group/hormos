//! Abstraction du moteur de conteneurs.
//!
//! Le cœur et les interfaces ne connaissent que ce trait ; l'implémentation
//! Docker vit dans `hormos-docker` (voir `docs/adr/0001-runtime-abstraction.md`).
//!
//! Le trait est **object-safe** : le cœur manipule un `Arc<dyn ContainerRuntime>`,
//! ce qui permet d'y brancher un `DockerRuntime`, un faux déterministe pour les
//! tests, ou plus tard une autre implémentation, sans changer une ligne des
//! interfaces. Les méthodes étant asynchrones, `#[async_trait]` est utilisé : il
//! produit exactement la désucrarisation `Pin<Box<dyn Future>>` qu'il faudrait
//! écrire à la main pour rendre un trait async utilisable derrière `dyn`.

use async_trait::async_trait;

use crate::domain::{ContainerDetails, ContainerSummary, SystemInfo};
use crate::error::Result;
use crate::reference::ContainerRef;

/// Opérations que doit fournir un moteur de conteneurs.
///
/// Les implémentations reçoivent une [`ContainerRef`] **déjà validée** : elles
/// n'ont pas à revalider l'entrée utilisateur.
#[async_trait]
pub trait ContainerRuntime: std::fmt::Debug + Send + Sync {
    /// Informations essentielles sur le moteur.
    async fn system_info(&self) -> Result<SystemInfo>;

    /// Liste les conteneurs : en cours d'exécution seulement, ou tous.
    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerSummary>>;

    /// Détail minimal d'un conteneur.
    async fn inspect_container(&self, reference: &ContainerRef) -> Result<ContainerDetails>;

    /// Démarre un conteneur. Idempotent : un conteneur déjà démarré est un succès.
    async fn start_container(&self, reference: &ContainerRef) -> Result<()>;

    /// Arrête un conteneur. Idempotent : un conteneur déjà arrêté est un succès.
    async fn stop_container(&self, reference: &ContainerRef) -> Result<()>;

    /// Redémarre un conteneur.
    async fn restart_container(&self, reference: &ContainerRef) -> Result<()>;
}

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
use crate::events::RuntimeEvent;
use crate::logs::{LogChunk, LogOptions};
use crate::reference::ContainerRef;
use crate::stream::RuntimeStream;

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

    /// Ouvre le journal d'un conteneur.
    ///
    /// La méthode est **synchrone** : elle ne fait que préparer le flux, sans
    /// attendre le moteur. Rien n'est demandé tant que le flux n'est pas consommé,
    /// et détruire le flux libère la ressource sous-jacente.
    ///
    /// Le flux **n'est pas borné par le délai d'attente** des opérations
    /// ponctuelles : un suivi (`follow`) doit pouvoir rester ouvert des heures.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si le flux ne peut pas être préparé. Une panne survenant
    /// **pendant** le flux est livrée comme élément de celui-ci, pas ici.
    fn container_logs(
        &self,
        reference: &ContainerRef,
        options: &LogOptions,
    ) -> Result<RuntimeStream<LogChunk>>;

    /// S'abonne au flux d'événements du moteur.
    ///
    /// Mêmes propriétés que [`ContainerRuntime::container_logs`] : préparation
    /// synchrone, ouverture paresseuse, annulation par destruction, aucun délai
    /// maximal.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si l'abonnement ne peut pas être préparé.
    fn runtime_events(&self) -> Result<RuntimeStream<RuntimeEvent>>;
}

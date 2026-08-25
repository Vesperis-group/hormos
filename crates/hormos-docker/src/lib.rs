//! Adaptateur Docker d'Hormos.
//!
//! Implémente [`hormos_core::ContainerRuntime`] au-dessus de
//! [Bollard](https://docs.rs/bollard), sur **socket local uniquement**.
//!
//! Garanties de cette crate :
//!
//! - **aucun transport distant**. Bollard est compilé avec
//!   `default-features = false, features = ["pipe"]` : les transports HTTP, TLS,
//!   SSH et WebSocket ne sont même pas présents dans le binaire. En complément,
//!   [`endpoint`] refuse explicitement tout `DOCKER_HOST` non `unix://` ;
//! - **aucun sous-processus** : ni `docker`, ni `sh -c`, ni interpolation shell.
//!   Toutes les opérations passent par l'API Engine ;
//! - **aucun type Bollard** n'est exposé : les réponses sont traduites vers le
//!   domaine d'`hormos-core` et les structures Bollard ne sont pas conservées ;
//! - **aucune opération sans délai maximal** (voir [`timeouts`]).

pub mod endpoint;
mod error;
mod mapping;
mod runtime;
pub mod timeouts;

pub use endpoint::LocalEndpoint;
pub use runtime::DockerRuntime;

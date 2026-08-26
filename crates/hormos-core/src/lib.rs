//! Cœur d'Hormos : « One engine. Every interface. »
//!
//! Cette crate contient tout ce qui est commun aux interfaces (CLI aujourd'hui ;
//! TUI, API, Web plus tard) :
//!
//! - le [domaine](domain) (types Hormos, indépendants de tout moteur) ;
//! - la [validation](reference) des références de conteneur ;
//! - le [modèle d'erreurs](error) ;
//! - les [flux temps réel](stream), les [journaux](logs) et les
//!   [événements](events) ;
//! - l'abstraction [`ContainerRuntime`](runtime::ContainerRuntime) ;
//! - le [service](service::ContainerService) qui expose les cas d'usage.
//!
//! Elle ne dépend **d'aucun** client Docker : aucun type Bollard n'apparaît ici,
//! ni dans l'API publique (voir `docs/adr/0001-runtime-abstraction.md`).

pub mod display;
pub mod domain;
pub mod error;
pub mod events;
pub mod logs;
pub mod reference;
pub mod runtime;
pub mod service;
pub mod stream;

/// Faux moteur déterministe, destiné aux **tests** des interfaces.
///
/// Activé automatiquement pour les tests de cette crate, et exposé aux autres
/// crates du workspace via la feature `mock` (déclarée en `dev-dependencies`
/// uniquement : aucun binaire de production ne l'embarque).
#[cfg(any(test, feature = "mock"))]
pub mod mock;

pub use error::{HormosError, Result};
pub use events::{ResourceKind, RuntimeEvent};
pub use logs::{LogChunk, LogDecoder, LogFramer, LogOptions, LogSource, LogTail};
pub use reference::ContainerRef;
pub use runtime::ContainerRuntime;
pub use service::ContainerService;
pub use stream::RuntimeStream;

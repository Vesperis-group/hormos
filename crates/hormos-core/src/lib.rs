//! Cœur d'Hormos : « One engine. Every interface. »
//!
//! Cette crate contient tout ce qui est commun aux interfaces (CLI aujourd'hui ;
//! TUI, API, Web plus tard) :
//!
//! - le [domaine](domain) (types Hormos, indépendants de tout moteur) ;
//! - la [validation](reference) des références de conteneur ;
//! - le [modèle d'erreurs](error) ;
//! - l'abstraction [`ContainerRuntime`](runtime::ContainerRuntime) ;
//! - le [service](service::ContainerService) qui expose les cas d'usage.
//!
//! Elle ne dépend **d'aucun** client Docker : aucun type Bollard n'apparaît ici,
//! ni dans l'API publique (voir `docs/adr/0001-runtime-abstraction.md`).

pub mod display;
pub mod domain;
pub mod error;
pub mod reference;
pub mod runtime;
pub mod service;

#[cfg(test)]
mod mock;

pub use error::{HormosError, Result};
pub use reference::ContainerRef;
pub use runtime::ContainerRuntime;
pub use service::ContainerService;

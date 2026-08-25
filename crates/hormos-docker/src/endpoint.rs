//! Résolution du point de terminaison Docker **local**.
//!
//! Hormos ne se connecte qu'à un socket Unix local. Cette phase ne supporte
//! **aucun** transport distant : si `DOCKER_HOST` désigne `tcp://`, `http://`,
//! `https://`, `ssh://` ou `npipe://`, Hormos échoue avec
//! [`HormosError::UnsupportedRuntime`] au lieu d'ouvrir silencieusement une
//! connexion réseau.
//!
//! Ordre de résolution :
//!
//! 1. `DOCKER_HOST` s'il est défini → doit être `unix:///chemin` ;
//! 2. `$XDG_RUNTIME_DIR/docker.sock` s'il existe (Docker **rootless**) ;
//! 3. `/var/run/docker.sock` (installation standard, y compris Docker Desktop
//!    avec intégration WSL).
//!
//! La découverte par défaut de Bollard (`connect_with_unix_defaults`) n'est pas
//! utilisée : elle ignore silencieusement un `DOCKER_HOST` distant et retombe
//! sur `/var/run/docker.sock`, ce qui masquerait précisément l'erreur que nous
//! voulons signaler, et elle ne connaît pas le socket rootless.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use hormos_core::error::{HormosError, Result};

/// Socket Docker standard.
pub const DEFAULT_SOCKET_PATH: &str = "/var/run/docker.sock";

/// Nom du socket dans le répertoire d'exécution utilisateur (Docker rootless).
const ROOTLESS_SOCKET_NAME: &str = "docker.sock";

/// Préfixe du seul schéma accepté.
const UNIX_SCHEME: &str = "unix://";

/// Point de terminaison local validé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEndpoint {
    path: PathBuf,
}

impl LocalEndpoint {
    /// Résout le point de terminaison depuis l'environnement du processus.
    ///
    /// # Errors
    ///
    /// [`HormosError::UnsupportedRuntime`] si `DOCKER_HOST` désigne un transport
    /// non supporté dans cette phase, [`HormosError::InvalidInput`] si
    /// `DOCKER_HOST` ou `XDG_RUNTIME_DIR` est défini mais illisible.
    pub fn from_env() -> Result<Self> {
        Self::resolve_os(
            std::env::var_os("DOCKER_HOST").as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
            |p| p.exists(),
        )
    }

    /// Résolution pure à partir des valeurs **brutes** du système.
    ///
    /// Une variable explicitement définie mais illisible est une erreur, jamais
    /// une absence : sans cela, un `DOCKER_HOST` non UTF-8 serait silencieusement
    /// remplacé par le socket standard, c'est-à-dire par un **autre démon** que
    /// celui que l'opérateur a configuré. La lecture échoue donc avant toute
    /// tentative de repli.
    ///
    /// # Errors
    ///
    /// Voir [`LocalEndpoint::from_env`].
    fn resolve_os(
        docker_host: Option<&OsStr>,
        runtime_dir: Option<&OsStr>,
        exists: impl Fn(&Path) -> bool,
    ) -> Result<Self> {
        let docker_host = readable("DOCKER_HOST", docker_host)?;
        let runtime_dir = readable("XDG_RUNTIME_DIR", runtime_dir)?;
        Self::resolve(docker_host, runtime_dir, exists)
    }

    /// Résolution pure, testable sans toucher à l'environnement réel.
    ///
    /// `exists` permet d'injecter la vérification de présence du socket.
    ///
    /// # Errors
    ///
    /// [`HormosError::UnsupportedRuntime`] si `docker_host` n'est pas un socket
    /// Unix local, [`HormosError::InvalidInput`] s'il est vide ou malformé.
    pub fn resolve(
        docker_host: Option<&str>,
        runtime_dir: Option<&str>,
        exists: impl Fn(&Path) -> bool,
    ) -> Result<Self> {
        if let Some(raw) = docker_host {
            let host = raw.trim();
            if !host.is_empty() {
                return Self::from_docker_host(host);
            }
        }

        if let Some(dir) = runtime_dir.map(str::trim).filter(|d| !d.is_empty()) {
            let candidate = Path::new(dir).join(ROOTLESS_SOCKET_NAME);
            if exists(&candidate) {
                return Ok(Self { path: candidate });
            }
        }

        Ok(Self {
            path: PathBuf::from(DEFAULT_SOCKET_PATH),
        })
    }

    fn from_docker_host(host: &str) -> Result<Self> {
        let Some(path) = host.strip_prefix(UNIX_SCHEME) else {
            let scheme = host.split_once("://").map_or("(sans schéma)", |(s, _)| s);
            return Err(HormosError::UnsupportedRuntime {
                detail: format!(
                    "DOCKER_HOST utilise le transport « {scheme} » ; \
                     cette version d'Hormos ne se connecte qu'à un socket Unix local (unix://…)"
                ),
            });
        };

        if path.is_empty() {
            return Err(HormosError::invalid_input(
                "DOCKER_HOST ne contient aucun chemin de socket après « unix:// »",
            ));
        }
        if path.contains('\0') {
            return Err(HormosError::invalid_input(
                "DOCKER_HOST contient un caractère NUL",
            ));
        }

        Ok(Self {
            path: PathBuf::from(path),
        })
    }

    /// Chemin du socket.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Chemin du socket sous forme de chaîne, pour Bollard et l'affichage.
    ///
    /// # Errors
    ///
    /// [`HormosError::InvalidInput`] si le chemin n'est pas de l'UTF-8 valide.
    pub fn as_str(&self) -> Result<&str> {
        self.path.to_str().ok_or_else(|| {
            HormosError::invalid_input("le chemin du socket Docker n'est pas de l'UTF-8 valide")
        })
    }
}

/// Lit une variable d'environnement en **échouant si elle est illisible**.
///
/// `std::env::var(..).ok()` confondrait « absente » et « présente mais non
/// UTF-8 » : la seconde deviendrait un repli silencieux vers un autre socket.
/// La valeur brute n'est jamais réaffichée — elle peut contenir n'importe quoi.
///
/// Bollard n'accepte qu'un chemin de socket UTF-8 (`connect_with_unix(&str, …)`) :
/// refuser ici n'est donc pas une limitation arbitraire.
fn readable<'a>(name: &str, raw: Option<&'a OsStr>) -> Result<Option<&'a str>> {
    match raw {
        None => Ok(None),
        Some(value) => value.to_str().map(Some).ok_or_else(|| {
            HormosError::invalid_input(format!("{name} n'est pas une chaîne UTF-8 valide"))
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{DEFAULT_SOCKET_PATH, LocalEndpoint};
    use hormos_core::error::ErrorKind;

    fn never(_: &Path) -> bool {
        false
    }

    fn always(_: &Path) -> bool {
        true
    }

    #[test]
    fn falls_back_to_default_socket() {
        let endpoint = LocalEndpoint::resolve(None, None, never).map(|e| e.path().to_owned());
        assert_eq!(endpoint, Ok(PathBuf::from(DEFAULT_SOCKET_PATH)));
    }

    #[test]
    fn prefers_rootless_socket_when_present() {
        let endpoint = LocalEndpoint::resolve(None, Some("/run/user/1000"), always)
            .map(|e| e.path().to_owned());
        assert_eq!(endpoint, Ok(PathBuf::from("/run/user/1000/docker.sock")));
    }

    #[test]
    fn ignores_rootless_socket_when_absent() {
        let endpoint = LocalEndpoint::resolve(None, Some("/run/user/1000"), never)
            .map(|e| e.path().to_owned());
        assert_eq!(endpoint, Ok(PathBuf::from(DEFAULT_SOCKET_PATH)));
    }

    #[test]
    fn accepts_custom_unix_socket() {
        let endpoint = LocalEndpoint::resolve(Some("unix:///tmp/custom.sock"), None, never)
            .map(|e| e.path().to_owned());
        assert_eq!(endpoint, Ok(PathBuf::from("/tmp/custom.sock")));
    }

    #[test]
    fn docker_host_wins_over_rootless() {
        let endpoint = LocalEndpoint::resolve(
            Some("unix:///tmp/custom.sock"),
            Some("/run/user/1000"),
            always,
        )
        .map(|e| e.path().to_owned());
        assert_eq!(endpoint, Ok(PathBuf::from("/tmp/custom.sock")));
    }

    #[test]
    fn rejects_every_remote_transport() {
        for host in [
            "tcp://10.0.0.1:2375",
            "http://10.0.0.1:2375",
            "https://10.0.0.1:2376",
            "ssh://user@10.0.0.1",
            "npipe:////./pipe/docker_engine",
            "fd://",
            "/var/run/docker.sock",
        ] {
            let kind = LocalEndpoint::resolve(Some(host), None, always)
                .map(|_| ())
                .map_err(|e| e.kind());
            assert_eq!(
                kind,
                Err(ErrorKind::UnsupportedRuntime),
                "transport accepté à tort : {host}"
            );
        }
    }

    #[test]
    fn rejects_empty_or_hostile_unix_path() {
        for host in ["unix://", "unix://\0/tmp/x"] {
            let kind = LocalEndpoint::resolve(Some(host), None, always)
                .map(|_| ())
                .map_err(|e| e.kind());
            assert_eq!(
                kind,
                Err(ErrorKind::InvalidInput),
                "accepté à tort : {host}"
            );
        }
    }

    #[test]
    fn blank_docker_host_falls_back() {
        let endpoint =
            LocalEndpoint::resolve(Some("   "), None, never).map(|e| e.path().to_owned());
        assert_eq!(endpoint, Ok(PathBuf::from(DEFAULT_SOCKET_PATH)));
    }

    /// Variables d'environnement brutes, telles que le système les fournit.
    ///
    /// Les tests passent par [`LocalEndpoint::resolve_os`] plutôt que par
    /// `std::env::set_var` : muter l'environnement du processus est visible par
    /// tous les tests exécutés en parallèle, et `set_var` est `unsafe` depuis
    /// l'édition 2024 — or `unsafe` est interdit dans ce dépôt.
    #[cfg(unix)]
    mod environment {
        use std::ffi::{OsStr, OsString};
        use std::os::unix::ffi::OsStringExt;
        use std::path::{Path, PathBuf};

        use super::{DEFAULT_SOCKET_PATH, LocalEndpoint, always, never};
        use hormos_core::error::ErrorKind;

        /// Valeur volontairement illisible : `0xFF` n'est jamais de l'UTF-8.
        fn invalid_utf8() -> OsString {
            OsString::from_vec(vec![b'u', b'n', b'i', b'x', b':', b'/', b'/', 0xFF])
        }

        fn resolve(
            docker_host: Option<&OsStr>,
            runtime_dir: Option<&OsStr>,
            exists: fn(&Path) -> bool,
        ) -> Result<PathBuf, ErrorKind> {
            LocalEndpoint::resolve_os(docker_host, runtime_dir, exists)
                .map(|e| e.path().to_owned())
                .map_err(|e| e.kind())
        }

        #[test]
        fn absent_variables_fall_back_to_the_default_socket() {
            assert_eq!(
                resolve(None, None, never),
                Ok(PathBuf::from(DEFAULT_SOCKET_PATH))
            );
        }

        #[test]
        fn a_valid_unix_docker_host_is_honoured() {
            let host = OsString::from("unix:///tmp/docker.sock");
            assert_eq!(
                resolve(Some(&host), None, never),
                Ok(PathBuf::from("/tmp/docker.sock"))
            );
        }

        #[test]
        fn a_remote_docker_host_is_still_refused() {
            let host = OsString::from("tcp://10.0.0.1:2375");
            assert_eq!(
                resolve(Some(&host), None, always),
                Err(ErrorKind::UnsupportedRuntime)
            );
        }

        #[test]
        fn an_unreadable_docker_host_fails_closed() {
            assert_eq!(
                resolve(Some(&invalid_utf8()), None, always),
                Err(ErrorKind::InvalidInput)
            );
        }

        #[test]
        fn an_unreadable_runtime_dir_fails_closed() {
            assert_eq!(
                resolve(None, Some(&invalid_utf8()), always),
                Err(ErrorKind::InvalidInput)
            );
        }

        /// Le point critique : une variable explicite mais illisible ne doit
        /// jamais dégénérer en connexion au socket standard, c'est-à-dire à un
        /// démon que l'opérateur n'a pas choisi.
        #[test]
        fn an_unreadable_variable_never_falls_back() {
            let runtime_dir = OsString::from("/run/user/1000");
            for (host, dir) in [
                (Some(invalid_utf8()), None),
                (Some(invalid_utf8()), Some(runtime_dir.clone())),
                (None, Some(invalid_utf8())),
            ] {
                let outcome = resolve(host.as_deref(), dir.as_deref(), always);
                assert_eq!(
                    outcome,
                    Err(ErrorKind::InvalidInput),
                    "repli silencieux après une variable illisible"
                );
            }
        }

        #[test]
        fn the_error_never_repeats_the_raw_value() {
            let Err(error) = LocalEndpoint::resolve_os(Some(&invalid_utf8()), None, always) else {
                panic!("une valeur illisible a été acceptée");
            };
            let message = error.to_string();
            assert!(
                message.contains("DOCKER_HOST") && !message.contains('\u{fffd}'),
                "message inattendu : {message}"
            );
        }
    }
}

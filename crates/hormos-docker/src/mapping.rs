//! Traduction des réponses Docker vers le domaine d'Hormos.
//!
//! Fonctions **pures** : elles ne touchent ni au réseau ni au socket, et sont
//! donc testables sans démon. Les structures Bollard ne sont jamais conservées
//! au-delà de la conversion.
//!
//! Rappel de sécurité : les variables d'environnement du conteneur ne sont ni
//! lues ni copiées ici. Elles contiennent régulièrement des secrets et n'ont
//! aucune raison d'apparaître dans un `inspect` minimal.

use bollard::container::LogOutput;
use bollard::models::{
    ContainerInspectResponse, ContainerState as DockerState, ContainerSummary as DockerSummary,
    EventMessage, EventMessageTypeEnum, SystemInfo as DockerSystemInfo,
};
use hormos_core::domain::{ContainerDetails, ContainerState, ContainerSummary, SystemInfo};
use hormos_core::events::{ResourceKind, RuntimeEvent};
use hormos_core::logs::{LogChunk, LogSource};

/// Valeur affichée lorsque le moteur n'a pas fourni le champ.
const UNKNOWN: &str = "<inconnu>";

/// Normalise un nom de conteneur Docker.
///
/// L'API renvoie les noms préfixés d'un `/` (`/hormos-test`) ; le domaine
/// conserve la forme sans préfixe, telle qu'un utilisateur la saisit.
#[must_use]
pub(crate) fn normalize_name(raw: &str) -> String {
    raw.strip_prefix('/').unwrap_or(raw).to_owned()
}

/// Choisit le nom d'affichage d'un conteneur listé.
///
/// Docker peut renvoyer plusieurs noms (alias réseau) : on retient le premier,
/// et l'identifiant tronqué si la liste est vide.
fn display_name(names: Option<&Vec<String>>, id: &str) -> String {
    names
        .and_then(|names| names.first())
        .map(|name| normalize_name(name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| short_id(id))
}

/// Identifiant tronqué à 12 caractères, comme le fait Docker.
#[must_use]
pub(crate) fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Traduit un élément de `GET /containers/json`.
#[must_use]
pub(crate) fn to_summary(summary: DockerSummary) -> ContainerSummary {
    let id = summary.id.unwrap_or_default();
    let state = summary.state.map_or_else(
        || ContainerState::Other(UNKNOWN.to_owned()),
        |state| ContainerState::from_engine(state.as_ref()),
    );

    ContainerSummary {
        name: display_name(summary.names.as_ref(), &id),
        image: summary.image.unwrap_or_else(|| UNKNOWN.to_owned()),
        state,
        status: summary.status.unwrap_or_default(),
        created: summary.created,
        id,
    }
}

/// Traduit une réponse de `GET /containers/{id}/json`.
#[must_use]
pub(crate) fn to_details(response: ContainerInspectResponse) -> ContainerDetails {
    let id = response.id.unwrap_or_default();
    let name = response
        .name
        .as_deref()
        .map(normalize_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| short_id(&id));

    let state = response
        .state
        .as_ref()
        .and_then(|state| state.status.as_ref())
        .map_or_else(
            || ContainerState::Other(UNKNOWN.to_owned()),
            |status| ContainerState::from_engine(status.as_ref()),
        );

    ContainerDetails {
        id,
        name,
        image: response.image.unwrap_or_else(|| UNKNOWN.to_owned()),
        status: response.state.as_ref().and_then(human_status),
        state,
        created: response.created,
        // Uniquement le nom d'hôte : le reste de `config` (dont `env`) est ignoré.
        hostname: response
            .config
            .and_then(|config| config.hostname)
            .filter(|hostname| !hostname.is_empty()),
        restart_count: response.restart_count,
    }
}

/// Résumé lisible de l'état détaillé, sans copier toute la structure Docker.
fn human_status(state: &DockerState) -> Option<String> {
    if state.running == Some(true) {
        return state
            .started_at
            .as_ref()
            .filter(|started| !started.is_empty())
            .map(|started| format!("Up since {started}"));
    }
    state
        .exit_code
        .map(|code| format!("Exited ({code})"))
        .or_else(|| state.status.as_ref().map(ToString::to_string))
}

/// Traduit `GET /info` en informations système Hormos.
///
/// `api_version` provient de la version **négociée** par le client, et non d'un
/// aller-retour supplémentaire.
#[must_use]
pub(crate) fn to_system_info(
    info: DockerSystemInfo,
    api_version: String,
    endpoint: String,
) -> SystemInfo {
    SystemInfo {
        server_version: info.server_version.filter(|v| !v.is_empty()),
        api_version: Some(api_version),
        os: info.os_type.filter(|v| !v.is_empty()),
        architecture: info.architecture.filter(|v| !v.is_empty()),
        containers_total: info.containers,
        containers_running: info.containers_running,
        endpoint,
    }
}

/// Traduit un fragment de journal Bollard vers le domaine.
///
/// Les octets sont recopiés tels quels : ni décodage, ni assainissement, ni
/// découpage. Ces trois responsabilités appartiennent au rendu (voir
/// [`hormos_core::logs`]), afin que toutes les interfaces partagent exactement la
/// même politique.
///
/// `StdIn` est replié sur [`LogSource::Console`] : le moteur ne l'émet que pour
/// un conteneur attaché à un terminal, où les sorties ne sont pas séparées.
#[must_use]
pub(crate) fn to_log_chunk(output: LogOutput) -> LogChunk {
    let (source, message) = match output {
        LogOutput::StdOut { message } => (LogSource::Stdout, message),
        LogOutput::StdErr { message } => (LogSource::Stderr, message),
        LogOutput::Console { message } | LogOutput::StdIn { message } => {
            (LogSource::Console, message)
        }
    };
    LogChunk::new(source, message.to_vec())
}

/// Traduit un événement Bollard vers le domaine.
///
/// **Seul** l'attribut `name` de l'acteur est lu. Le moteur place dans ce même
/// dictionnaire l'intégralité des labels de la ressource, qui contiennent en
/// pratique des jetons de déploiement et des chaînes de connexion : ne pas les
/// traduire est la garantie qu'ils ne peuvent être ni affichés, ni sérialisés.
#[must_use]
pub(crate) fn to_runtime_event(message: EventMessage) -> RuntimeEvent {
    let (actor_id, actor_name) = message.actor.map_or((None, None), |actor| {
        let name = actor
            .attributes
            .and_then(|attributes| attributes.get("name").cloned())
            .filter(|name| !name.is_empty());
        (actor.id.filter(|id| !id.is_empty()), name)
    });

    RuntimeEvent {
        timestamp: message.time,
        kind: to_resource_kind(message.typ),
        action: message.action.unwrap_or_else(|| UNKNOWN.to_owned()),
        actor_id,
        actor_name,
    }
}

/// Traduit la catégorie d'un événement.
///
/// Tout ce que le domaine n'expose pas encore est replié sur
/// [`ResourceKind::Other`] plutôt que d'ajouter des variantes spéculatives.
fn to_resource_kind(kind: Option<EventMessageTypeEnum>) -> ResourceKind {
    match kind {
        Some(EventMessageTypeEnum::CONTAINER) => ResourceKind::Container,
        Some(EventMessageTypeEnum::IMAGE) => ResourceKind::Image,
        Some(EventMessageTypeEnum::VOLUME) => ResourceKind::Volume,
        Some(EventMessageTypeEnum::NETWORK) => ResourceKind::Network,
        _ => ResourceKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bollard::container::LogOutput;
    use bollard::models::{
        ContainerConfig, ContainerInspectResponse, ContainerState as DockerState,
        ContainerStateStatusEnum, ContainerSummary as DockerSummary, ContainerSummaryStateEnum,
        EventActor, EventMessage, EventMessageTypeEnum, SystemInfo as DockerSystemInfo,
    };
    use hormos_core::domain::ContainerState;
    use hormos_core::events::ResourceKind;
    use hormos_core::logs::LogSource;

    use super::{
        normalize_name, short_id, to_details, to_log_chunk, to_runtime_event, to_summary,
        to_system_info,
    };

    fn summary() -> DockerSummary {
        DockerSummary {
            id: Some("0123456789abcdef0123".to_owned()),
            names: Some(vec!["/hormos-test".to_owned()]),
            image: Some("alpine:3.22".to_owned()),
            state: Some(ContainerSummaryStateEnum::RUNNING),
            status: Some("Up 2 hours".to_owned()),
            created: Some(1_700_000_000),
            ..Default::default()
        }
    }

    #[test]
    fn normalizes_leading_slash_in_names() {
        assert_eq!(normalize_name("/hormos-test"), "hormos-test");
        assert_eq!(normalize_name("hormos-test"), "hormos-test");
        assert_eq!(normalize_name("/a/b"), "a/b");
    }

    #[test]
    fn summary_is_mapped_completely() {
        let mapped = to_summary(summary());
        assert_eq!(mapped.id, "0123456789abcdef0123");
        assert_eq!(mapped.name, "hormos-test");
        assert_eq!(mapped.image, "alpine:3.22");
        assert_eq!(mapped.state, ContainerState::Running);
        assert_eq!(mapped.status, "Up 2 hours");
        assert_eq!(mapped.created, Some(1_700_000_000));
    }

    #[test]
    fn summary_keeps_the_full_id_and_truncates_only_for_display() {
        let mapped = to_summary(summary());
        assert_eq!(mapped.id.chars().count(), 20);
        assert_eq!(short_id(&mapped.id), "0123456789ab");
    }

    #[test]
    fn summary_without_names_falls_back_to_short_id() {
        let mapped = to_summary(DockerSummary {
            names: None,
            ..summary()
        });
        assert_eq!(mapped.name, "0123456789ab");

        let mapped = to_summary(DockerSummary {
            names: Some(Vec::new()),
            ..summary()
        });
        assert_eq!(mapped.name, "0123456789ab");
    }

    #[test]
    fn summary_without_state_is_marked_unknown() {
        let mapped = to_summary(DockerSummary {
            state: None,
            ..summary()
        });
        assert_eq!(mapped.state, ContainerState::Other("<inconnu>".to_owned()));
        assert!(!mapped.state.is_running());
    }

    fn inspect() -> ContainerInspectResponse {
        ContainerInspectResponse {
            id: Some("0123456789abcdef0123".to_owned()),
            name: Some("/hormos-test".to_owned()),
            image: Some("sha256:abc".to_owned()),
            created: Some("2026-01-01T00:00:00Z".to_owned()),
            restart_count: Some(3),
            state: Some(DockerState {
                status: Some(ContainerStateStatusEnum::RUNNING),
                running: Some(true),
                started_at: Some("2026-01-01T01:00:00Z".to_owned()),
                ..Default::default()
            }),
            config: Some(ContainerConfig {
                hostname: Some("box".to_owned()),
                env: Some(vec!["SECRET_TOKEN=hunter2".to_owned()]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn details_are_mapped_completely() {
        let mapped = to_details(inspect());
        assert_eq!(mapped.id, "0123456789abcdef0123");
        assert_eq!(mapped.name, "hormos-test");
        assert_eq!(mapped.image, "sha256:abc");
        assert_eq!(mapped.state, ContainerState::Running);
        assert_eq!(
            mapped.status.as_deref(),
            Some("Up since 2026-01-01T01:00:00Z")
        );
        assert_eq!(mapped.created.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(mapped.hostname.as_deref(), Some("box"));
        assert_eq!(mapped.restart_count, Some(3));
    }

    #[test]
    fn details_never_carry_environment_variables() {
        // Le domaine n'a aucun champ d'environnement : la sérialisation d'un
        // inspect contenant un secret ne peut pas le laisser fuir.
        let mapped = to_details(inspect());
        let rendered = format!("{mapped:?}");
        assert!(
            !rendered.contains("SECRET_TOKEN"),
            "secret recopié : {rendered}"
        );
        assert!(!rendered.contains("hunter2"), "secret recopié : {rendered}");
    }

    #[test]
    fn stopped_details_report_the_exit_code() {
        let mapped = to_details(ContainerInspectResponse {
            state: Some(DockerState {
                status: Some(ContainerStateStatusEnum::EXITED),
                running: Some(false),
                exit_code: Some(137),
                ..Default::default()
            }),
            ..inspect()
        });
        assert_eq!(mapped.state, ContainerState::Exited);
        assert_eq!(mapped.status.as_deref(), Some("Exited (137)"));
    }

    #[test]
    fn details_without_name_fall_back_to_short_id() {
        let mapped = to_details(ContainerInspectResponse {
            name: None,
            ..inspect()
        });
        assert_eq!(mapped.name, "0123456789ab");
    }

    #[test]
    fn system_info_is_mapped_and_uses_the_negotiated_api_version() {
        let mapped = to_system_info(
            DockerSystemInfo {
                server_version: Some("29.7.2".to_owned()),
                os_type: Some("linux".to_owned()),
                architecture: Some("x86_64".to_owned()),
                containers: Some(7),
                containers_running: Some(2),
                ..Default::default()
            },
            "1.51".to_owned(),
            "/var/run/docker.sock".to_owned(),
        );
        assert_eq!(mapped.server_version.as_deref(), Some("29.7.2"));
        assert_eq!(mapped.api_version.as_deref(), Some("1.51"));
        assert_eq!(mapped.os.as_deref(), Some("linux"));
        assert_eq!(mapped.architecture.as_deref(), Some("x86_64"));
        assert_eq!(mapped.containers_total, Some(7));
        assert_eq!(mapped.containers_running, Some(2));
        assert_eq!(mapped.endpoint, "/var/run/docker.sock");
    }

    #[test]
    fn empty_engine_strings_become_none() {
        let mapped = to_system_info(
            DockerSystemInfo {
                server_version: Some(String::new()),
                os_type: Some(String::new()),
                ..Default::default()
            },
            "1.51".to_owned(),
            "socket".to_owned(),
        );
        assert_eq!(mapped.server_version, None);
        assert_eq!(mapped.os, None);
    }

    #[test]
    fn log_chunks_keep_their_bytes_untouched() {
        // Séquence ANSI + octet UTF-8 invalide : le mapping ne doit rien altérer,
        // l'assainissement est la responsabilité du rendu.
        let hostile = vec![0x1b, b'[', b'2', b'K', 0xff, b'\n'];
        let chunk = to_log_chunk(LogOutput::StdOut {
            message: hostile.clone().into(),
        });
        assert_eq!(chunk.source, LogSource::Stdout);
        assert_eq!(chunk.data, hostile);
    }

    #[test]
    fn log_sources_are_mapped_including_the_tty_case() {
        let cases = [
            (
                LogOutput::StdOut {
                    message: b"a".to_vec().into(),
                },
                LogSource::Stdout,
            ),
            (
                LogOutput::StdErr {
                    message: b"a".to_vec().into(),
                },
                LogSource::Stderr,
            ),
            (
                LogOutput::Console {
                    message: b"a".to_vec().into(),
                },
                LogSource::Console,
            ),
            // En mode tty le moteur n'a pas de sortie séparée : `StdIn` est replié.
            (
                LogOutput::StdIn {
                    message: b"a".to_vec().into(),
                },
                LogSource::Console,
            ),
        ];
        for (output, expected) in cases {
            assert_eq!(to_log_chunk(output).source, expected);
        }
    }

    #[test]
    fn an_empty_log_chunk_stays_empty() {
        let chunk = to_log_chunk(LogOutput::StdErr {
            message: Vec::new().into(),
        });
        assert!(chunk.data.is_empty());
    }

    fn event(attributes: HashMap<String, String>) -> EventMessage {
        EventMessage {
            typ: Some(EventMessageTypeEnum::CONTAINER),
            action: Some("start".to_owned()),
            actor: Some(EventActor {
                id: Some("0123456789abcdef".to_owned()),
                attributes: Some(attributes),
            }),
            time: Some(1_700_000_000),
            ..Default::default()
        }
    }

    #[test]
    fn events_keep_only_the_name_attribute() {
        let attributes = HashMap::from([
            ("name".to_owned(), "web".to_owned()),
            // Labels typiques d'un déploiement : ils ne doivent pas survivre.
            ("com.example.token".to_owned(), "s3cr3t".to_owned()),
            ("image".to_owned(), "registry.internal/app:1".to_owned()),
        ]);
        let mapped = to_runtime_event(event(attributes));

        assert_eq!(mapped.kind, ResourceKind::Container);
        assert_eq!(mapped.action, "start");
        assert_eq!(mapped.actor_name.as_deref(), Some("web"));
        assert_eq!(mapped.actor_id.as_deref(), Some("0123456789abcdef"));
        assert_eq!(mapped.timestamp, Some(1_700_000_000));

        // Le type du domaine n'a aucun champ où un label pourrait se glisser.
        let rendered = format!("{mapped:?}");
        assert!(
            !rendered.contains("s3cr3t"),
            "un label a fuité : {rendered}"
        );
        assert!(!rendered.contains("registry.internal"));
    }

    #[test]
    fn events_tolerate_missing_fields() {
        let mapped = to_runtime_event(EventMessage::default());
        assert_eq!(mapped.kind, ResourceKind::Other);
        assert_eq!(mapped.action, "<inconnu>");
        assert_eq!(mapped.actor_id, None);
        assert_eq!(mapped.actor_name, None);
        assert_eq!(mapped.timestamp, None);
    }

    #[test]
    fn empty_event_strings_become_none() {
        let mut message = event(HashMap::from([("name".to_owned(), String::new())]));
        if let Some(actor) = message.actor.as_mut() {
            actor.id = Some(String::new());
        }
        let mapped = to_runtime_event(message);
        assert_eq!(mapped.actor_id, None);
        assert_eq!(mapped.actor_name, None);
    }

    #[test]
    fn event_kinds_fall_back_to_other() {
        let cases = [
            (EventMessageTypeEnum::CONTAINER, ResourceKind::Container),
            (EventMessageTypeEnum::IMAGE, ResourceKind::Image),
            (EventMessageTypeEnum::VOLUME, ResourceKind::Volume),
            (EventMessageTypeEnum::NETWORK, ResourceKind::Network),
            (EventMessageTypeEnum::DAEMON, ResourceKind::Other),
            (EventMessageTypeEnum::SECRET, ResourceKind::Other),
            (EventMessageTypeEnum::EMPTY, ResourceKind::Other),
        ];
        for (typ, expected) in cases {
            let message = EventMessage {
                typ: Some(typ),
                ..Default::default()
            };
            assert_eq!(to_runtime_event(message).kind, expected);
        }
    }
}

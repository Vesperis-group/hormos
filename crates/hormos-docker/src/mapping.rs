//! Traduction des réponses Docker vers le domaine d'Hormos.
//!
//! Fonctions **pures** : elles ne touchent ni au réseau ni au socket, et sont
//! donc testables sans démon. Les structures Bollard ne sont jamais conservées
//! au-delà de la conversion.
//!
//! Rappel de sécurité : les variables d'environnement du conteneur ne sont ni
//! lues ni copiées ici. Elles contiennent régulièrement des secrets et n'ont
//! aucune raison d'apparaître dans un `inspect` minimal.

use bollard::models::{
    ContainerInspectResponse, ContainerState as DockerState, ContainerSummary as DockerSummary,
    SystemInfo as DockerSystemInfo,
};
use hormos_core::domain::{ContainerDetails, ContainerState, ContainerSummary, SystemInfo};

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

#[cfg(test)]
mod tests {
    use bollard::models::{
        ContainerConfig, ContainerInspectResponse, ContainerState as DockerState,
        ContainerStateStatusEnum, ContainerSummary as DockerSummary, ContainerSummaryStateEnum,
        SystemInfo as DockerSystemInfo,
    };
    use hormos_core::domain::ContainerState;

    use super::{normalize_name, short_id, to_details, to_summary, to_system_info};

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
}

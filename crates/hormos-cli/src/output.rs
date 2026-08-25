//! Rendu des résultats pour un terminal.
//!
//! Deux formats : texte lisible (par défaut) et JSON (`--json`). Dans les deux
//! cas, **toute** chaîne provenant du moteur passe par
//! [`hormos_core::display::sanitize`] : un nom de conteneur ou un nom d'image est
//! choisi par celui qui a créé le conteneur, donc non fiable.
//!
//! L'identifiant complet est conservé dans le domaine ; il n'est tronqué qu'ici,
//! à l'affichage, comme le fait Docker.

use hormos_core::display::{sanitize, sanitize_truncated};
use hormos_core::domain::{ContainerDetails, ContainerSummary, SystemInfo};

/// Longueur d'affichage d'un identifiant de conteneur.
const SHORT_ID_LEN: usize = 12;

/// Largeur maximale d'une colonne de texte libre.
const COLUMN_MAX: usize = 40;

/// Valeur affichée quand le moteur n'a pas fourni l'information.
const UNKNOWN: &str = "<inconnu>";

/// Sérialise une valeur en JSON indenté.
///
/// # Errors
///
/// Renvoie l'erreur de `serde_json` si la sérialisation échoue.
pub fn to_json<T: serde::Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}

/// Rendu texte des informations moteur.
#[must_use]
pub fn render_info(info: &SystemInfo) -> String {
    let rows = [
        ("Point de terminaison", sanitize(&info.endpoint)),
        (
            "Version du serveur",
            optional(info.server_version.as_deref()),
        ),
        ("Version de l'API", optional(info.api_version.as_deref())),
        ("Système", optional(info.os.as_deref())),
        ("Architecture", optional(info.architecture.as_deref())),
        ("Conteneurs", count(info.containers_total)),
        ("En exécution", count(info.containers_running)),
    ];

    let label_width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);

    rows.iter()
        .map(|(label, value)| format!("{label:<label_width$}  {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rendu texte d'une liste de conteneurs.
#[must_use]
pub fn render_list(containers: &[ContainerSummary]) -> String {
    let headers = ["ID", "NOM", "IMAGE", "ÉTAT", "STATUT"];
    let mut rows: Vec<[String; 5]> = vec![headers.map(ToOwned::to_owned)];

    rows.extend(containers.iter().map(|container| {
        [
            short_id(&container.id),
            sanitize_truncated(&container.name, COLUMN_MAX),
            sanitize_truncated(&container.image, COLUMN_MAX),
            sanitize_truncated(container.state.as_str(), COLUMN_MAX),
            sanitize_truncated(&container.status, COLUMN_MAX),
        ]
    }));

    let widths: Vec<usize> = (0..headers.len())
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    rows.iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(column, cell)| pad(cell, widths[column]))
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rendu texte du détail d'un conteneur.
#[must_use]
pub fn render_details(details: &ContainerDetails) -> String {
    let rows = [
        ("Identifiant", sanitize(&details.id)),
        ("Nom", sanitize(&details.name)),
        ("Image", sanitize(&details.image)),
        ("État", sanitize(details.state.as_str())),
        ("Statut", optional(details.status.as_deref())),
        ("Créé le", optional(details.created.as_deref())),
        ("Nom d'hôte", optional(details.hostname.as_deref())),
        ("Redémarrages", count(details.restart_count)),
    ];

    let label_width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);

    rows.iter()
        .map(|(label, value)| format!("{label:<label_width$}  {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Complète une cellule à la largeur voulue (comptage en caractères).
fn pad(cell: &str, width: usize) -> String {
    let padding = width.saturating_sub(cell.chars().count());
    let mut out = cell.to_owned();
    out.extend(std::iter::repeat_n(' ', padding));
    out
}

/// Identifiant raccourci pour l'affichage, comme le fait Docker.
///
/// Le domaine conserve l'identifiant complet : la troncature est purement
/// cosmétique et n'a pas de suffixe, pour rester copiable-collable.
fn short_id(id: &str) -> String {
    sanitize(id).chars().take(SHORT_ID_LEN).collect()
}

fn optional(value: Option<&str>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), sanitize)
}

fn count(value: Option<i64>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |v| v.to_string())
}

#[cfg(test)]
mod tests {
    use hormos_core::domain::{ContainerDetails, ContainerState, ContainerSummary, SystemInfo};

    use super::{render_details, render_info, render_list, to_json};

    fn summary(name: &str, image: &str) -> ContainerSummary {
        ContainerSummary {
            id: "0123456789abcdef0123456789abcdef".to_owned(),
            name: name.to_owned(),
            image: image.to_owned(),
            state: ContainerState::Running,
            status: "Up 2 hours".to_owned(),
            created: Some(1_700_000_000),
        }
    }

    #[test]
    fn list_has_a_header_and_one_line_per_container() {
        let rendered = render_list(&[summary("web", "alpine:3.22"), summary("db", "postgres:17")]);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("ID"));
        assert!(lines[1].contains("web"));
        assert!(lines[2].contains("db"));
    }

    #[test]
    fn empty_list_still_shows_the_header() {
        let rendered = render_list(&[]);
        assert_eq!(rendered.lines().count(), 1);
        assert!(rendered.contains("STATUT"));
    }

    #[test]
    fn identifiers_are_truncated_only_for_display() {
        let container = summary("web", "alpine:3.22");
        let rendered = render_list(std::slice::from_ref(&container));
        assert!(rendered.contains("0123456789ab"));
        assert!(!rendered.contains("0123456789abc"));
        assert_eq!(container.id.chars().count(), 32, "le domaine a été tronqué");
    }

    #[test]
    fn hostile_names_cannot_drive_the_terminal() {
        let rendered = render_list(&[summary("web\u{1b}[2K\u{7}", "alp\rine")]);
        assert!(!rendered.contains('\u{1b}'), "séquence ANSI conservée");
        assert!(!rendered.contains('\u{7}'), "BEL conservé");
        assert!(!rendered.contains('\r'), "retour chariot conservé");
        assert_eq!(rendered.lines().count(), 2, "ligne injectée dans la sortie");
    }

    #[test]
    fn hostile_names_cannot_inject_lines_in_details() {
        let details = ContainerDetails {
            id: "abc".to_owned(),
            name: "web\nÉtat  compromis".to_owned(),
            image: "alpine".to_owned(),
            state: ContainerState::Running,
            status: None,
            created: None,
            hostname: None,
            restart_count: None,
        };
        let rendered = render_details(&details);
        assert_eq!(rendered.lines().count(), 8, "ligne injectée dans la sortie");
    }

    #[test]
    fn missing_fields_are_explicit() {
        let info = SystemInfo {
            server_version: None,
            api_version: None,
            os: None,
            architecture: None,
            containers_total: None,
            containers_running: None,
            endpoint: "/var/run/docker.sock".to_owned(),
        };
        let rendered = render_info(&info);
        assert!(rendered.contains("/var/run/docker.sock"));
        assert!(rendered.contains("<inconnu>"));
        assert_eq!(rendered.lines().count(), 7);
    }

    #[test]
    fn json_output_is_stable_and_parsable() {
        let container = summary("web", "alpine:3.22");
        let json = to_json(&container).unwrap_or_default();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
        assert_eq!(parsed["name"], "web");
        assert_eq!(parsed["id"], "0123456789abcdef0123456789abcdef");
        assert_eq!(parsed["state"], "running");
    }

    #[test]
    fn json_details_never_expose_environment_variables() {
        let details = ContainerDetails {
            id: "abc".to_owned(),
            name: "web".to_owned(),
            image: "alpine".to_owned(),
            state: ContainerState::Running,
            status: None,
            created: None,
            hostname: Some("box".to_owned()),
            restart_count: Some(0),
        };
        let json = to_json(&details).unwrap_or_default();
        assert!(!json.to_lowercase().contains("env"));
    }
}

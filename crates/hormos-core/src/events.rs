//! Événements du moteur de conteneurs.
//!
//! Le moteur publie un flux d'événements décrivant ce qui se passe sur l'hôte :
//! un conteneur démarre, une image est supprimée, un volume est créé. Hormos n'en
//! conserve qu'une projection **minimale et volontairement pauvre**.
//!
//! # Ce qui est délibérément écarté
//!
//! Un événement Docker transporte un dictionnaire d'attributs qui reprend **tous
//! les labels** de la ressource concernée. Ces labels sont fixés par celui qui a
//! créé le conteneur et contiennent en pratique des jetons de déploiement, des
//! chaînes de connexion ou des chemins internes. Hormos ne lit donc **que**
//! l'attribut `name` et ignore tout le reste : ce qui n'est pas modélisé ici ne
//! peut être ni affiché, ni journalisé, ni sérialisé par erreur.

use serde::Serialize;

/// Horodatage de repli lorsqu'une valeur ne peut pas être représentée.
const UNKNOWN_TIME: &str = "-";

/// Catégorie de ressource concernée par un événement.
///
/// Les catégories que le domaine n'expose pas encore sont regroupées sous
/// [`ResourceKind::Other`] plutôt que d'être inventées à l'avance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    /// Un conteneur.
    Container,
    /// Une image.
    Image,
    /// Un volume.
    Volume,
    /// Un réseau.
    Network,
    /// Toute autre catégorie (démon, greffon, secret, service…).
    #[default]
    Other,
}

impl ResourceKind {
    /// Libellé court et stable, utilisable en sortie machine.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Image => "image",
            Self::Volume => "volume",
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

/// Événement du moteur, réduit à ce que les interfaces affichent.
///
/// Les champs textuels proviennent du moteur et restent **non assainis** dans le
/// domaine, conformément à la règle générale : c'est le rendu qui assainit (voir
/// [`crate::display`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeEvent {
    /// Horodatage UNIX en secondes, si le moteur l'a fourni.
    pub timestamp: Option<i64>,
    /// Catégorie de ressource concernée.
    pub kind: ResourceKind,
    /// Action observée (`start`, `die`, `pull`…), telle que nommée par le moteur.
    pub action: String,
    /// Identifiant de la ressource, si connu.
    pub actor_id: Option<String>,
    /// Nom de la ressource, seul attribut retenu du moteur.
    pub actor_name: Option<String>,
}

impl RuntimeEvent {
    /// Forme abrégée de l'identifiant, comme l'affichent les outils de conteneurs.
    ///
    /// La coupe est faite sur les **caractères** et non les octets : elle ne peut
    /// donc pas produire de l'UTF-8 invalide, même si le moteur renvoyait un
    /// identifiant inattendu.
    #[must_use]
    pub fn short_id(&self, length: usize) -> Option<String> {
        self.actor_id
            .as_ref()
            .map(|id| id.chars().take(length).collect())
    }

    /// Horodatage lisible, en UTC, ou `-` si le moteur ne l'a pas fourni.
    #[must_use]
    pub fn formatted_time(&self) -> String {
        self.timestamp
            .map_or_else(|| UNKNOWN_TIME.to_owned(), format_timestamp)
    }
}

/// Formate un horodatage UNIX en date UTC `AAAA-MM-JJTHH:MM:SSZ`.
///
/// Le calcul est fait ici plutôt qu'avec une bibliothèque de dates : la
/// conversion « jours depuis l'époque → date civile » tient en quelques lignes,
/// elle est exacte pour tout l'intervalle utile, et elle évite d'ajouter une
/// dépendance — donc une surface de chaîne d'approvisionnement — pour afficher
/// une colonne.
///
/// Une valeur hors de l'intervalle représentable (année en dehors de
/// `0001`..=`9999`) est rendue telle quelle, en secondes : mieux vaut une valeur
/// brute qu'une date fausse.
#[must_use]
pub fn format_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (
        time_of_day / 3_600,
        (time_of_day / 60) % 60,
        time_of_day % 60,
    );

    // Algorithme « civil_from_days » de Howard Hinnant : l'époque est décalée au
    // 1er mars de l'an 0 pour que l'année bissextile tombe en fin de cycle.
    let Some(shifted) = days.checked_add(719_468) else {
        return seconds.to_string();
    };
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    if !(1..=9_999).contains(&year) {
        return seconds.to_string();
    }
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::{ResourceKind, RuntimeEvent, format_timestamp};

    fn event(id: Option<&str>) -> RuntimeEvent {
        RuntimeEvent {
            timestamp: Some(1_700_000_000),
            kind: ResourceKind::Container,
            action: "start".to_owned(),
            actor_id: id.map(str::to_owned),
            actor_name: Some("web".to_owned()),
        }
    }

    #[test]
    fn kind_labels_are_stable() {
        let cases = [
            (ResourceKind::Container, "container"),
            (ResourceKind::Image, "image"),
            (ResourceKind::Volume, "volume"),
            (ResourceKind::Network, "network"),
            (ResourceKind::Other, "other"),
        ];
        for (kind, expected) in cases {
            assert_eq!(kind.as_str(), expected);
        }
        assert_eq!(ResourceKind::default(), ResourceKind::Other);
    }

    #[test]
    fn short_id_truncates_on_characters() {
        assert_eq!(
            event(Some("0123456789ab")).short_id(4).as_deref(),
            Some("0123")
        );
        assert_eq!(event(Some("ab")).short_id(12).as_deref(), Some("ab"));
        assert_eq!(event(None).short_id(12), None);
        // Un identifiant non ASCII ne doit pas être coupé au milieu d'un caractère.
        assert_eq!(event(Some("日本語")).short_id(2).as_deref(), Some("日本"));
    }

    #[test]
    fn timestamps_are_formatted_in_utc() {
        let cases = [
            (0_i64, "1970-01-01T00:00:00Z"),
            (1_700_000_000, "2023-11-14T22:13:20Z"),
            // 29 février d'une année bissextile divisible par 400.
            (951_782_400, "2000-02-29T00:00:00Z"),
            // 28 février d'une année divisible par 100 mais pas par 400.
            (4_107_456_000, "2100-02-28T00:00:00Z"),
            (-1, "1969-12-31T23:59:59Z"),
            (-86_400, "1969-12-31T00:00:00Z"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(format_timestamp(seconds), expected, "pour {seconds}");
        }
    }

    #[test]
    fn unrepresentable_timestamps_fall_back_to_raw_seconds() {
        for seconds in [i64::MIN, i64::MAX, -100_000_000_000_000] {
            assert_eq!(format_timestamp(seconds), seconds.to_string());
        }
    }

    #[test]
    fn a_missing_timestamp_is_displayed_as_a_dash() {
        let mut sample = event(Some("id"));
        assert_eq!(sample.formatted_time(), "2023-11-14T22:13:20Z");
        sample.timestamp = None;
        assert_eq!(sample.formatted_time(), "-");
    }
}

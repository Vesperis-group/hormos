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

/// Catégorie de ressource concernée par un événement.
///
/// Les catégories que le domaine n'expose pas encore sont regroupées sous
/// [`ResourceKind::Other`] plutôt que d'être inventées à l'avance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

#[cfg(test)]
mod tests {
    use super::{ResourceKind, RuntimeEvent};

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
}

//! Référence de conteneur validée.
//!
//! Une référence est soit un **nom**, soit un **identifiant** (complet ou
//! préfixe). Elle est validée **avant** tout appel au moteur : une entrée
//! invalide n'atteint jamais le réseau ni le socket.
//!
//! La validation n'est pas cosmétique. L'API Docker construit ses chemins sous
//! la forme `/containers/{reference}/json` : une référence contenant `/`, `?`,
//! `#` ou `%` pourrait déplacer la requête vers un autre chemin de l'API. Les
//! caractères de contrôle (dont `NUL`) sont refusés pour la même raison, et
//! parce qu'ils sont dangereux à l'affichage.
//!
//! Le jeu autorisé correspond à celui des noms Docker (`[a-zA-Z0-9][a-zA-Z0-9_.-]*`),
//! qui couvre également les identifiants hexadécimaux : il n'exclut donc aucune
//! référence réellement acceptée par le moteur.

use std::fmt;

use crate::error::{HormosError, Result};

/// Longueur maximale acceptée pour une référence.
///
/// Un identifiant Docker fait 64 caractères et les noms sont largement plus
/// courts. La borne évite qu'une entrée arbitrairement longue soit relayée au
/// moteur.
pub const MAX_REFERENCE_LEN: usize = 128;

/// Référence de conteneur garantie valide par construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContainerRef(String);

impl ContainerRef {
    /// Valide et construit une référence.
    ///
    /// # Errors
    ///
    /// Retourne [`HormosError::InvalidInput`] si la référence est vide, faite
    /// uniquement d'espaces, trop longue, ou contient un caractère hors du jeu
    /// autorisé (caractères de contrôle, `NUL`, `/`, espaces, etc.).
    pub fn new(value: &str) -> Result<Self> {
        if value.is_empty() {
            return Err(HormosError::invalid_input(
                "la référence de conteneur est vide",
            ));
        }
        if value.trim().is_empty() {
            return Err(HormosError::invalid_input(
                "la référence de conteneur ne contient que des espaces",
            ));
        }
        if value.chars().count() > MAX_REFERENCE_LEN {
            return Err(HormosError::invalid_input(format!(
                "la référence de conteneur dépasse {MAX_REFERENCE_LEN} caractères"
            )));
        }

        let mut chars = value.chars();
        let first = chars
            .next()
            .ok_or_else(|| HormosError::invalid_input("la référence de conteneur est vide"))?;
        if !first.is_ascii_alphanumeric() {
            return Err(HormosError::invalid_input(
                "la référence de conteneur doit commencer par une lettre ou un chiffre",
            ));
        }
        for c in chars {
            if !is_allowed(c) {
                return Err(HormosError::invalid_input(
                    "la référence de conteneur ne peut contenir que des lettres, chiffres, « _ », « . » et « - »",
                ));
            }
        }

        Ok(Self(value.to_owned()))
    }

    /// Retourne la référence validée.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContainerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const fn is_allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

#[cfg(test)]
mod tests {
    use super::{ContainerRef, MAX_REFERENCE_LEN};
    use crate::error::ErrorKind;

    #[test]
    fn accepts_names_and_ids() {
        for value in [
            "nginx",
            "hormos-test-abc123",
            "my_app.1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "a",
        ] {
            let reference = ContainerRef::new(value).map(|r| r.as_str().to_owned());
            assert_eq!(reference.as_deref(), Ok(value), "refusé à tort : {value}");
        }
    }

    #[test]
    fn rejects_empty_and_whitespace() {
        for value in ["", " ", "\t", "   \n  "] {
            let kind = ContainerRef::new(value).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(kind, Err(ErrorKind::InvalidInput), "accepté à tort");
        }
    }

    #[test]
    fn rejects_nul_and_control_characters() {
        for value in ["ngi\u{0}nx", "ngi\u{1b}[31mnx", "ngi\nnx", "ngi\u{7f}nx"] {
            let kind = ContainerRef::new(value).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(kind, Err(ErrorKind::InvalidInput), "accepté à tort");
        }
    }

    #[test]
    fn rejects_path_and_url_metacharacters() {
        for value in [
            "../../images/json",
            "nginx/json",
            "nginx?all=1",
            "nginx#frag",
            "nginx%2f",
            "ngi nx",
        ] {
            let kind = ContainerRef::new(value).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(
                kind,
                Err(ErrorKind::InvalidInput),
                "accepté à tort : {value}"
            );
        }
    }

    #[test]
    fn rejects_leading_non_alphanumeric() {
        for value in ["-rf", ".hidden", "_x", "/nginx"] {
            let kind = ContainerRef::new(value).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(
                kind,
                Err(ErrorKind::InvalidInput),
                "accepté à tort : {value}"
            );
        }
    }

    #[test]
    fn rejects_overlong_reference() {
        let value = "a".repeat(MAX_REFERENCE_LEN + 1);
        let kind = ContainerRef::new(&value).map(|_| ()).map_err(|e| e.kind());
        assert_eq!(kind, Err(ErrorKind::InvalidInput));
    }

    #[test]
    fn accepts_maximum_length() {
        let value = "a".repeat(MAX_REFERENCE_LEN);
        assert!(ContainerRef::new(&value).is_ok());
    }
}

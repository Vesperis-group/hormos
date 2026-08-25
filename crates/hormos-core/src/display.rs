//! Assainissement des chaînes **non fiables** renvoyées par le moteur.
//!
//! Les noms de conteneur, noms d'image, statuts et labels sont contrôlés par
//! celui qui a créé le conteneur. Affichés tels quels dans un terminal, ils
//! peuvent contenir des séquences d'échappement ANSI (`\x1b[…`), des retours
//! chariot ou des caractères de contrôle capables de réécrire l'affichage, de
//! masquer des lignes ou de piloter l'émulateur de terminal.
//!
//! Le domaine conserve les valeurs telles que renvoyées par le moteur ; c'est au
//! **rendu** de les assainir. Cette fonction est fournie ici pour que toutes les
//! interfaces partagent exactement la même politique.

/// Remplace tout caractère de contrôle par `U+FFFD` (caractère de remplacement).
///
/// Sont neutralisés : les contrôles C0 (`U+0000`..=`U+001F`, dont `NUL`, `ESC`,
/// `CR`, `LF`, `BEL`), `DEL` (`U+007F`) et les contrôles C1 (`U+0080`..=`U+009F`,
/// qui incluent la forme 8 bits de `ESC`). Le reste du texte, y compris
/// l'Unicode imprimable, est conservé intact.
#[must_use]
pub fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if is_control(c) { '\u{fffd}' } else { c })
        .collect()
}

/// Assainit puis tronque à `max_chars` caractères, en suffixant `…` si tronqué.
///
/// La troncature s'applique aux **caractères** (pas aux octets) : elle ne peut
/// donc pas couper au milieu d'un point de code UTF-8.
#[must_use]
pub fn sanitize_truncated(value: &str, max_chars: usize) -> String {
    let sanitized = sanitize(value);
    if sanitized.chars().count() <= max_chars {
        return sanitized;
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = sanitized.chars().take(keep).collect();
    out.push('…');
    out
}

const fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
}

#[cfg(test)]
mod tests {
    use super::{sanitize, sanitize_truncated};

    #[test]
    fn strips_ansi_escape_sequences() {
        let hostile = "nginx\u{1b}[2Ktricked";
        assert_eq!(sanitize(hostile), "nginx\u{fffd}[2Ktricked");
    }

    #[test]
    fn strips_newlines_carriage_returns_and_nul() {
        assert_eq!(sanitize("a\nb\rc\u{0}d"), "a\u{fffd}b\u{fffd}c\u{fffd}d");
    }

    #[test]
    fn strips_c1_controls() {
        // U+009B est la forme 8 bits de CSI.
        assert_eq!(sanitize("x\u{9b}31m"), "x\u{fffd}31m");
    }

    #[test]
    fn keeps_printable_unicode() {
        assert_eq!(sanitize("café-日本-🙂"), "café-日本-🙂");
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(sanitize_truncated("日本語テスト", 3), "日本…");
        assert_eq!(sanitize_truncated("abc", 3), "abc");
        assert_eq!(sanitize_truncated("abcd", 3), "ab…");
    }
}

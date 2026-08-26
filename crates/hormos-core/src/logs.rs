//! Journaux de conteneur : options, fragments et rendu sûr.
//!
//! Un journal est du **contenu hostile** : il est écrit par le processus qui
//! tourne dans le conteneur, donc potentiellement par du code non maîtrisé. Il
//! peut contenir des séquences d'échappement ANSI capables de réécrire l'écran,
//! des octets qui ne forment pas de l'UTF-8 valide, ou une « ligne » de plusieurs
//! gigaoctets sans le moindre retour à la ligne.
//!
//! Ce module fournit les deux primitives partagées par toutes les interfaces :
//!
//! - [`LogDecoder`] — décodage UTF-8 **incrémental** et assainissement, à mémoire
//!   quasi constante ; il ne bufferise jamais une ligne.
//! - [`LogFramer`] — découpage en lignes **bornées**, construit au-dessus du
//!   décodeur, pour les interfaces qui affichent ligne par ligne.
//!
//! # Politique d'assainissement
//!
//! Contrairement à [`crate::display::sanitize`], qui neutralise *tous* les
//! caractères de contrôle, la politique des journaux **conserve `\n` et `\t`** :
//! sans eux un journal n'est plus lisible. Tout le reste — `ESC`, `BEL`, `NUL`,
//! les contrôles C1, `DEL` — est remplacé par `U+FFFD`.
//!
//! `\r` est traité à part : la paire `\r\n` devient `\n` (les journaux produits
//! par un programme d'origine Windows restent lisibles), tandis qu'un `\r` isolé
//! est remplacé par `U+FFFD`, car il permettrait de réécrire la ligne courante du
//! terminal.

use crate::error::{HormosError, Result};

/// Caractère de remplacement Unicode, substitué à tout octet ou contrôle refusé.
const REPLACEMENT: char = '\u{fffd}';

/// Nombre maximal d'octets qu'une séquence UTF-8 tronquée peut occuper en report.
const MAX_CARRY: usize = 3;

/// Borne haute acceptée pour `--tail`.
///
/// Au-delà, la demande est refusée plutôt que transmise au moteur : `tail` fait
/// remonter le moteur dans le fichier de journal, et une valeur démesurée s'y
/// traduit par une lecture massive côté démon.
pub const MAX_TAIL_LINES: u32 = 100_000;

/// Longueur maximale par défaut d'une ligne affichée, en octets.
pub const DEFAULT_MAX_LINE_BYTES: usize = 16 * 1024;

/// Marque ajoutée en fin de ligne lorsque celle-ci a été tronquée.
pub const TRUNCATION_MARKER: &str = "…[ligne tronquée]";

/// Sortie standard d'origine d'un fragment de journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogSource {
    /// Sortie standard du processus.
    Stdout,
    /// Sortie d'erreur du processus.
    Stderr,
    /// Flux unique d'un conteneur attaché à un terminal (mode `tty`).
    ///
    /// Dans ce mode le moteur ne sépare pas les deux sorties : tout arrive
    /// mélangé sur un seul canal.
    Console,
}

impl LogSource {
    /// Libellé court et stable, utilisable en sortie machine.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Console => "console",
        }
    }
}

/// Fragment brut de journal, tel que livré par le moteur.
///
/// Les octets ne sont **ni décodés ni assainis** : un fragment peut couper un
/// caractère UTF-8 en deux, ou ne contenir aucun retour à la ligne. C'est à
/// l'interface d'appliquer [`LogDecoder`] ou [`LogFramer`] avant tout affichage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogChunk {
    /// Sortie d'origine.
    pub source: LogSource,
    /// Octets bruts.
    pub data: Vec<u8>,
}

impl LogChunk {
    /// Construit un fragment.
    #[must_use]
    pub fn new(source: LogSource, data: Vec<u8>) -> Self {
        Self { source, data }
    }
}

/// Quantité d'historique demandée avant le début du suivi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogTail {
    /// Tout l'historique disponible.
    #[default]
    All,
    /// Les `n` dernières lignes.
    Lines(u32),
}

impl LogTail {
    /// Nombre de lignes demandé, ou `None` pour « tout l'historique ».
    #[must_use]
    pub const fn lines(self) -> Option<u32> {
        match self {
            Self::All => None,
            Self::Lines(count) => Some(count),
        }
    }

    /// Interprète une valeur fournie par l'utilisateur : `all` ou un entier.
    ///
    /// # Errors
    ///
    /// Renvoie [`HormosError::InvalidInput`] si la valeur n'est ni `all` ni un
    /// entier, ou si elle dépasse [`MAX_TAIL_LINES`].
    pub fn parse(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        if trimmed.eq_ignore_ascii_case("all") {
            return Ok(Self::All);
        }
        let count: u32 = trimmed.parse().map_err(|_| {
            HormosError::invalid_input("--tail attend « all » ou un entier positif")
        })?;
        if count > MAX_TAIL_LINES {
            return Err(HormosError::invalid_input(format!(
                "--tail est limité à {MAX_TAIL_LINES} lignes"
            )));
        }
        Ok(Self::Lines(count))
    }
}

/// Options de lecture d'un journal de conteneur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogOptions {
    /// Continuer à suivre le journal après l'historique.
    pub follow: bool,
    /// Historique demandé avant le suivi.
    pub tail: LogTail,
    /// Demander au moteur de préfixer chaque ligne d'un horodatage.
    pub timestamps: bool,
}

impl LogOptions {
    /// Options par défaut : tout l'historique, sans suivi ni horodatage.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Active ou non le suivi.
    #[must_use]
    pub const fn follow(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    /// Fixe l'historique demandé.
    #[must_use]
    pub const fn tail(mut self, tail: LogTail) -> Self {
        self.tail = tail;
        self
    }

    /// Active ou non l'horodatage côté moteur.
    #[must_use]
    pub const fn timestamps(mut self, timestamps: bool) -> Self {
        self.timestamps = timestamps;
        self
    }
}

/// Décodeur UTF-8 incrémental et assainissant, à mémoire quasi constante.
///
/// Il conserve au plus [`MAX_CARRY`] octets entre deux fragments (une séquence
/// UTF-8 coupée en fin de fragment) et un indicateur d'un `\r` en attente. Il
/// **n'accumule jamais de ligne** : un journal sans aucun retour à la ligne ne le
/// fait pas grossir.
///
/// Un décodeur est valable pour **une seule** sortie : `stdout` et `stderr`
/// arrivent entrelacés et leurs séquences UTF-8 ne doivent pas être recollées
/// entre elles. Il en faut donc un par [`LogSource`].
#[derive(Debug, Default)]
pub struct LogDecoder {
    carry: Vec<u8>,
    pending_carriage_return: bool,
}

impl LogDecoder {
    /// Crée un décodeur vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Décode et assainit un fragment.
    ///
    /// Le texte renvoyé est immédiatement affichable : il ne contient plus aucun
    /// caractère de contrôle en dehors de `\n` et `\t`.
    #[must_use]
    pub fn push(&mut self, data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len());
        if self.carry.is_empty() {
            self.decode(data, &mut out);
        } else {
            // Une séquence UTF-8 a été coupée par le fragment précédent : c'est
            // le seul cas où l'on recopie, et il porte sur 3 octets au plus.
            let mut joined = std::mem::take(&mut self.carry);
            joined.extend_from_slice(data);
            self.decode(&joined, &mut out);
        }
        out
    }

    /// Vide l'état résiduel en fin de flux.
    ///
    /// Une séquence UTF-8 incomplète ou un `\r` en attente sont alors rendus
    /// visibles sous la forme d'un `U+FFFD`, plutôt que silencieusement perdus.
    #[must_use]
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if self.pending_carriage_return {
            self.pending_carriage_return = false;
            out.push(REPLACEMENT);
        }
        if !self.carry.is_empty() {
            self.carry.clear();
            out.push(REPLACEMENT);
        }
        out
    }

    fn decode(&mut self, mut data: &[u8], out: &mut String) {
        loop {
            match std::str::from_utf8(data) {
                Ok(text) => {
                    self.sanitize_into(text, out);
                    return;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if let Ok(text) = std::str::from_utf8(&data[..valid]) {
                        self.sanitize_into(text, out);
                    }
                    match error.error_len() {
                        // Séquence réellement invalide : un remplacement, puis on
                        // reprend juste après.
                        Some(len) => {
                            out.push(REPLACEMENT);
                            data = &data[valid + len..];
                        }
                        // Séquence simplement tronquée par la fin du fragment :
                        // on la garde pour le fragment suivant.
                        None => {
                            let rest = &data[valid..];
                            if rest.len() <= MAX_CARRY {
                                self.carry.extend_from_slice(rest);
                            } else {
                                out.push(REPLACEMENT);
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    fn sanitize_into(&mut self, text: &str, out: &mut String) {
        for character in text.chars() {
            if self.pending_carriage_return {
                self.pending_carriage_return = false;
                if character == '\n' {
                    out.push('\n');
                    continue;
                }
                // `\r` isolé : il replacerait le curseur en début de ligne.
                out.push(REPLACEMENT);
            }
            match character {
                '\r' => self.pending_carriage_return = true,
                '\n' | '\t' => out.push(character),
                '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => out.push(REPLACEMENT),
                _ => out.push(character),
            }
        }
    }
}

/// Découpeur de journal en lignes **bornées**.
///
/// Construit au-dessus de [`LogDecoder`], il ajoute le découpage sur `\n` et une
/// limite de longueur : au-delà de `max_line_bytes`, le reste de la ligne est
/// abandonné et la ligne émise porte [`TRUNCATION_MARKER`]. Une ligne infinie ne
/// peut donc pas faire croître la mémoire.
#[derive(Debug)]
pub struct LogFramer {
    decoder: LogDecoder,
    current: String,
    truncated: bool,
    max_line_bytes: usize,
}

impl Default for LogFramer {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_LINE_BYTES)
    }
}

impl LogFramer {
    /// Crée un découpeur limitant chaque ligne à `max_line_bytes` octets.
    #[must_use]
    pub fn new(max_line_bytes: usize) -> Self {
        Self {
            decoder: LogDecoder::new(),
            current: String::new(),
            truncated: false,
            max_line_bytes: max_line_bytes.max(1),
        }
    }

    /// Consomme un fragment et appelle `on_line` pour **chaque ligne complète**.
    ///
    /// Les lignes sont livrées au fil de l'eau plutôt que collectées : un
    /// fragment ne contenant que des retours à la ligne ne provoque donc aucune
    /// accumulation intermédiaire.
    pub fn push(&mut self, data: &[u8], mut on_line: impl FnMut(String)) {
        let text = self.decoder.push(data);
        for character in text.chars() {
            if character == '\n' {
                on_line(self.take_line());
                continue;
            }
            self.push_char(character);
        }
    }

    /// Émet la ligne partielle éventuellement en attente, en fin de flux.
    #[must_use]
    pub fn flush(&mut self) -> Option<String> {
        let tail = self.decoder.finish();
        for character in tail.chars() {
            self.push_char(character);
        }
        if self.current.is_empty() && !self.truncated {
            return None;
        }
        Some(self.take_line())
    }

    fn push_char(&mut self, character: char) {
        if self.truncated {
            return;
        }
        if self.current.len() + character.len_utf8() > self.max_line_bytes {
            self.truncated = true;
            return;
        }
        self.current.push(character);
    }

    fn take_line(&mut self) -> String {
        let mut line = std::mem::take(&mut self.current);
        if self.truncated {
            self.truncated = false;
            line.push_str(TRUNCATION_MARKER);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{
        DEFAULT_MAX_LINE_BYTES, LogDecoder, LogFramer, LogOptions, LogSource, LogTail,
        MAX_TAIL_LINES, TRUNCATION_MARKER,
    };
    use crate::error::ErrorKind;

    fn lines(framer: &mut LogFramer, data: &[u8]) -> Vec<String> {
        let mut collected = Vec::new();
        framer.push(data, |line| collected.push(line));
        collected
    }

    #[test]
    fn tail_accepts_all_and_numbers() {
        assert_eq!(LogTail::parse("all").unwrap(), LogTail::All);
        assert_eq!(LogTail::parse("ALL").unwrap(), LogTail::All);
        assert_eq!(LogTail::parse(" 42 ").unwrap(), LogTail::Lines(42));
        assert_eq!(LogTail::parse("0").unwrap(), LogTail::Lines(0));
    }

    #[test]
    fn tail_rejects_garbage_and_excess() {
        for value in ["", "-1", "12x", "1e3", "٤٢"] {
            assert_eq!(
                LogTail::parse(value).unwrap_err().kind(),
                ErrorKind::InvalidInput,
                "« {value} » doit être refusé"
            );
        }
        let too_many = (u64::from(MAX_TAIL_LINES) + 1).to_string();
        assert_eq!(
            LogTail::parse(&too_many).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn default_options_are_conservative() {
        let options = LogOptions::new();
        assert!(!options.follow);
        assert!(!options.timestamps);
        assert_eq!(options.tail, LogTail::All);
        assert_eq!(options.tail.lines(), None);
        assert_eq!(LogTail::Lines(5).lines(), Some(5));
    }

    #[test]
    fn source_labels_are_stable() {
        assert_eq!(LogSource::Stdout.as_str(), "stdout");
        assert_eq!(LogSource::Stderr.as_str(), "stderr");
        assert_eq!(LogSource::Console.as_str(), "console");
    }

    #[test]
    fn decoder_neutralises_ansi_but_keeps_newline_and_tab() {
        let mut decoder = LogDecoder::new();
        let text = decoder.push(b"a\x1b[31mb\tc\nd\x07");
        assert_eq!(text, "a\u{fffd}[31mb\tc\nd\u{fffd}");
    }

    #[test]
    fn decoder_maps_crlf_to_newline_and_flags_lone_cr() {
        let mut decoder = LogDecoder::new();
        assert_eq!(decoder.push(b"a\r\nb\rc"), "a\nb\u{fffd}c");
    }

    #[test]
    fn decoder_handles_a_carriage_return_split_across_chunks() {
        let mut decoder = LogDecoder::new();
        assert_eq!(decoder.push(b"a\r"), "a");
        assert_eq!(decoder.push(b"\nb"), "\nb");

        let mut decoder = LogDecoder::new();
        assert_eq!(decoder.push(b"a\r"), "a");
        assert_eq!(decoder.push(b"b"), "\u{fffd}b");
    }

    #[test]
    fn decoder_flushes_a_trailing_carriage_return() {
        let mut decoder = LogDecoder::new();
        assert_eq!(decoder.push(b"x\r"), "x");
        assert_eq!(decoder.finish(), "\u{fffd}");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn decoder_reassembles_utf8_split_across_chunks() {
        let bytes = "é日🙂".as_bytes().to_vec();
        // Un octet à la fois : le pire découpage possible.
        let mut decoder = LogDecoder::new();
        let mut out = String::new();
        for byte in &bytes {
            out.push_str(&decoder.push(&[*byte]));
        }
        assert_eq!(out, "é日🙂");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn decoder_replaces_invalid_bytes_without_losing_the_rest() {
        let mut decoder = LogDecoder::new();
        assert_eq!(decoder.push(b"a\xffb"), "a\u{fffd}b");
        assert_eq!(decoder.push(&[0xe2, 0x28, 0xa1]), "\u{fffd}(\u{fffd}");
    }

    #[test]
    fn decoder_flushes_a_truncated_sequence() {
        let mut decoder = LogDecoder::new();
        assert_eq!(decoder.push(&[0xf0, 0x9f]), "");
        assert_eq!(decoder.finish(), "\u{fffd}");
    }

    #[test]
    fn decoder_stays_small_on_a_line_without_any_newline() {
        let mut decoder = LogDecoder::new();
        for _ in 0..64 {
            let text = decoder.push(&[b'x'; 4096]);
            assert_eq!(text.len(), 4096);
        }
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn framer_splits_on_newlines_only() {
        let mut framer = LogFramer::default();
        assert_eq!(lines(&mut framer, b"un\ndeux\ntro"), ["un", "deux"]);
        assert_eq!(lines(&mut framer, b"is\n"), ["trois"]);
        assert_eq!(framer.flush(), None);
    }

    #[test]
    fn framer_emits_empty_lines() {
        let mut framer = LogFramer::default();
        assert_eq!(lines(&mut framer, b"\n\n"), ["", ""]);
    }

    #[test]
    fn framer_flushes_a_partial_line() {
        let mut framer = LogFramer::default();
        assert!(lines(&mut framer, b"fin sans retour").is_empty());
        assert_eq!(framer.flush().unwrap(), "fin sans retour");
        assert_eq!(framer.flush(), None);
    }

    #[test]
    fn framer_truncates_a_giant_line_and_resumes_after_it() {
        let mut framer = LogFramer::new(8);
        let mut collected = Vec::new();
        for _ in 0..1_000 {
            framer.push(&[b'x'; 1_024], |line| collected.push(line));
        }
        assert!(
            collected.is_empty(),
            "aucune ligne complète n'est encore là"
        );
        framer.push(b"\nsuite\n", |line| collected.push(line));
        assert_eq!(
            collected,
            [format!("xxxxxxxx{TRUNCATION_MARKER}"), "suite".to_owned()]
        );
    }

    #[test]
    fn framer_truncation_is_char_safe() {
        let mut framer = LogFramer::new(4);
        // « é » fait 2 octets : la cinquième ne tient pas dans 4 octets.
        assert_eq!(
            lines(&mut framer, "ééé\n".as_bytes()),
            [format!("éé{TRUNCATION_MARKER}")]
        );
    }

    #[test]
    fn framer_sanitizes_like_the_decoder() {
        let mut framer = LogFramer::default();
        assert_eq!(
            lines(&mut framer, b"\x1b]0;titre\x07\r\nok\n"),
            ["\u{fffd}]0;titre\u{fffd}", "ok"]
        );
    }

    #[test]
    fn framer_flush_reports_a_truncated_trailing_line() {
        let mut framer = LogFramer::new(2);
        assert!(lines(&mut framer, b"abcdef").is_empty());
        assert_eq!(framer.flush().unwrap(), format!("ab{TRUNCATION_MARKER}"));
    }

    #[test]
    fn default_line_budget_is_sane() {
        assert_eq!(DEFAULT_MAX_LINE_BYTES, 16 * 1024);
    }
}

//! Tampons bornés des flux affichés par le TUI.
//!
//! Un flux suivi n'a pas de fin : il faut donc décider **à l'avance** ce que
//! l'on garde. Deux bornes plutôt qu'une : un nombre d'éléments, pour que le
//! défilement reste utilisable, et un volume d'octets, parce que deux mille
//! lignes de seize kilo-octets pèseraient trente-deux mégaoctets. La plus
//! stricte des deux l'emporte.
//!
//! L'ancre de défilement est comptée **depuis le début** du tampon et corrigée à
//! chaque éviction. Une ancre comptée depuis la fin donnerait l'illusion de la
//! stabilité tout en glissant d'une ligne à chaque nouvelle arrivée : c'est
//! exactement le défaut que ce module existe pour éviter.

use std::collections::VecDeque;

use hormos_core::logs::LogSource;

/// Nombre maximal de lignes de journal conservées.
pub const MAX_LOG_LINES: usize = 2_000;

/// Volume maximal de journal conservé, en octets.
pub const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;

/// Longueur maximale d'une ligne avant troncature par le découpeur.
pub const MAX_LINE_BYTES: usize = 16 * 1024;

/// Nombre maximal d'événements conservés.
pub const MAX_EVENTS: usize = 500;

/// Nombre de lignes de fin demandées à l'ouverture d'un journal.
///
/// Assez pour comprendre ce qui vient de se passer, assez peu pour que
/// l'ouverture reste instantanée sur un conteneur bavard depuis des jours.
pub const TAIL_ON_OPEN: u32 = 200;

/// Ligne de journal prête à l'affichage, déjà décodée et assainie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Sortie d'origine, qui décide de la couleur au rendu.
    pub source: LogSource,
    /// Texte de la ligne, sans retour à la ligne final.
    pub text: String,
}

impl LogLine {
    /// Construit une ligne attribuée à une sortie.
    #[must_use]
    pub fn new(source: LogSource, text: String) -> Self {
        Self { source, text }
    }

    /// Poids mémoire retenu pour la comptabilité du tampon.
    #[must_use]
    pub fn weight(&self) -> usize {
        self.text.len()
    }
}

/// État d'avancement du flux courant.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StreamState {
    /// Aucun flux ouvert.
    #[default]
    Idle,
    /// Flux demandé, rien reçu pour l'instant.
    Connecting,
    /// Flux ouvert et alimenté.
    Live,
    /// Flux terminé ; `Some` porte le message d'échec.
    Ended(Option<String>),
}

/// Élément conservé avec son poids, pour ne pas le recalculer à l'éviction.
#[derive(Debug)]
struct Entry<T> {
    item: T,
    bytes: usize,
}

/// Fenêtre bornée sur un flux, avec son défilement.
///
/// Le tampon ne connaît pas la hauteur de l'écran : celle-ci lui est passée à
/// chaque opération de défilement. Il reste donc purement testable, sans
/// terminal.
#[derive(Debug)]
pub struct Pane<T> {
    items: VecDeque<Entry<T>>,
    bytes: usize,
    max_items: usize,
    max_bytes: usize,
    dropped: u64,
    /// Indice de la première ligne affichée ; `None` colle le tampon au bas.
    anchor: Option<usize>,
}

impl<T> Pane<T> {
    /// Crée un tampon vide borné en éléments et en octets.
    #[must_use]
    pub const fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            items: VecDeque::new(),
            bytes: 0,
            max_items: if max_items == 0 { 1 } else { max_items },
            max_bytes,
            dropped: 0,
            anchor: None,
        }
    }

    /// Ajoute un élément et évince ce qui dépasse les bornes.
    pub fn push(&mut self, item: T, bytes: usize) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.items.push_back(Entry { item, bytes });
        self.evict();
    }

    /// Vide le tampon et revient au bas de l'écran.
    ///
    /// Le compteur d'éléments perdus est remis à zéro : il décrit le flux
    /// affiché, pas l'historique de la session.
    pub fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
        self.dropped = 0;
        self.anchor = None;
    }

    /// Nombre d'éléments conservés.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Indique que le tampon est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Nombre d'éléments évincés depuis l'ouverture du flux.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Indique que l'affichage suit la fin du flux.
    #[must_use]
    pub const fn follows(&self) -> bool {
        self.anchor.is_none()
    }

    /// Indice du premier élément affiché pour une hauteur donnée.
    #[must_use]
    pub fn first_visible(&self, viewport: usize) -> usize {
        let bottom = self.len().saturating_sub(viewport.max(1));
        self.anchor.map_or(bottom, |anchor| anchor.min(bottom))
    }

    /// Éléments affichés pour une hauteur donnée.
    pub fn visible(&self, viewport: usize) -> impl Iterator<Item = &T> {
        self.items
            .iter()
            .skip(self.first_visible(viewport))
            .take(viewport.max(1))
            .map(|entry| &entry.item)
    }

    /// Remonte de `amount` éléments et quitte le suivi de la fin.
    pub fn scroll_up(&mut self, amount: usize, viewport: usize) {
        let current = self.first_visible(viewport);
        self.anchor = Some(current.saturating_sub(amount.max(1)));
    }

    /// Redescend de `amount` éléments ; atteindre le bas rétablit le suivi.
    pub fn scroll_down(&mut self, amount: usize, viewport: usize) {
        let bottom = self.len().saturating_sub(viewport.max(1));
        let next = self.first_visible(viewport).saturating_add(amount.max(1));
        self.anchor = if next >= bottom { None } else { Some(next) };
    }

    /// Va au tout début du tampon conservé.
    pub const fn to_top(&mut self) {
        self.anchor = Some(0);
    }

    /// Revient à la fin et rétablit le suivi.
    pub const fn to_bottom(&mut self) {
        self.anchor = None;
    }

    /// Évince par le début tant qu'une borne est dépassée.
    ///
    /// L'ancre est décalée du même nombre d'éléments : la fenêtre reste sur le
    /// texte que l'utilisateur regarde, au lieu de glisser vers le bas.
    fn evict(&mut self) {
        let mut removed = 0_usize;
        while self.items.len() > self.max_items
            || (self.bytes > self.max_bytes && self.items.len() > 1)
        {
            let Some(entry) = self.items.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            self.dropped = self.dropped.saturating_add(1);
            removed = removed.saturating_add(1);
        }
        if removed > 0
            && let Some(anchor) = self.anchor
        {
            self.anchor = Some(anchor.saturating_sub(removed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LogLine, MAX_LOG_BYTES, Pane};
    use hormos_core::logs::LogSource;

    fn pane(max_items: usize) -> Pane<usize> {
        Pane::new(max_items, usize::MAX)
    }

    fn filled(count: usize) -> Pane<usize> {
        let mut pane = pane(1_000);
        for index in 0..count {
            pane.push(index, 0);
        }
        pane
    }

    fn shown(pane: &Pane<usize>, viewport: usize) -> Vec<usize> {
        pane.visible(viewport).copied().collect()
    }

    #[test]
    fn a_fresh_pane_shows_the_end_of_the_stream() {
        let pane = filled(100);
        assert!(pane.follows());
        assert_eq!(shown(&pane, 3), vec![97, 98, 99]);
    }

    #[test]
    fn a_short_stream_is_shown_whole() {
        let pane = filled(2);
        assert_eq!(shown(&pane, 10), vec![0, 1]);
    }

    #[test]
    fn the_oldest_items_are_evicted_first() {
        let mut pane = pane(3);
        for index in 0..5 {
            pane.push(index, 0);
        }

        assert_eq!(pane.len(), 3);
        assert_eq!(pane.dropped(), 2);
        assert_eq!(shown(&pane, 3), vec![2, 3, 4]);
    }

    #[test]
    fn the_byte_budget_evicts_before_the_item_budget() {
        let mut pane: Pane<usize> = Pane::new(1_000, 10);
        for index in 0..5 {
            pane.push(index, 4);
        }

        assert_eq!(pane.len(), 2);
        assert_eq!(pane.dropped(), 3);
    }

    #[test]
    fn a_single_oversized_item_is_kept_rather_than_dropped() {
        // Évincer le dernier élément viderait l'écran sans rien montrer de
        // l'erreur : mieux vaut dépasser la borne d'une ligne.
        let mut pane: Pane<usize> = Pane::new(1_000, 4);
        pane.push(0, 4_096);

        assert_eq!(pane.len(), 1);
        assert_eq!(pane.dropped(), 0);
    }

    #[test]
    fn scrolling_up_leaves_the_tail() {
        let mut pane = filled(100);
        pane.scroll_up(5, 10);

        assert!(!pane.follows());
        assert_eq!(pane.first_visible(10), 85);
    }

    #[test]
    fn scrolling_back_to_the_bottom_restores_following() {
        let mut pane = filled(100);
        pane.scroll_up(5, 10);
        pane.scroll_down(5, 10);

        assert!(pane.follows());
        assert_eq!(shown(&pane, 3), vec![97, 98, 99]);
    }

    #[test]
    fn scrolling_past_the_bottom_does_not_overshoot() {
        let mut pane = filled(100);
        pane.scroll_up(50, 10);
        pane.scroll_down(1_000, 10);

        assert!(pane.follows());
        assert_eq!(pane.first_visible(10), 90);
    }

    #[test]
    fn scrolling_above_the_start_stops_at_the_start() {
        let mut pane = filled(100);
        pane.scroll_up(1_000, 10);

        assert_eq!(pane.first_visible(10), 0);
        assert_eq!(shown(&pane, 3), vec![0, 1, 2]);
    }

    #[test]
    fn a_held_view_does_not_slide_when_new_items_arrive() {
        let mut pane = filled(100);
        pane.scroll_up(10, 10);
        let held = shown(&pane, 10);

        for index in 100..110 {
            pane.push(index, 0);
        }

        assert_eq!(shown(&pane, 10), held, "la fenêtre a glissé");
    }

    #[test]
    fn a_held_view_follows_its_content_through_eviction() {
        let mut pane = pane(20);
        for index in 0..20 {
            pane.push(index, 0);
        }
        pane.scroll_up(10, 5); // affiche 5..10
        assert_eq!(shown(&pane, 5), vec![5, 6, 7, 8, 9]);

        for index in 20..25 {
            pane.push(index, 0);
        }

        // Cinq éléments ont disparu du début : l'ancre a suivi, donc le texte
        // regardé est toujours le même.
        assert_eq!(shown(&pane, 5), vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn eviction_can_push_a_held_view_back_to_the_start() {
        let mut pane = pane(10);
        for index in 0..10 {
            pane.push(index, 0);
        }
        pane.scroll_up(1_000, 5);

        for index in 10..20 {
            pane.push(index, 0);
        }

        assert_eq!(pane.first_visible(5), 0);
        assert!(!pane.follows(), "le suivi ne se rétablit pas tout seul");
    }

    #[test]
    fn a_followed_pane_keeps_showing_the_newest_items() {
        let mut pane = pane(10);
        for index in 0..50 {
            pane.push(index, 0);
        }

        assert_eq!(shown(&pane, 3), vec![47, 48, 49]);
    }

    #[test]
    fn clearing_forgets_the_previous_stream() {
        let mut pane = pane(3);
        for index in 0..10 {
            pane.push(index, 0);
        }
        pane.scroll_up(2, 2);
        pane.clear();

        assert!(pane.is_empty());
        assert_eq!(pane.dropped(), 0);
        assert!(pane.follows());
    }

    #[test]
    fn a_zero_height_viewport_is_treated_as_one_line() {
        let pane = filled(10);
        assert_eq!(shown(&pane, 0), vec![9]);
    }

    #[test]
    fn log_lines_weigh_their_text() {
        let line = LogLine::new(LogSource::Stdout, "bonjour".to_owned());
        assert_eq!(line.weight(), 7);
        assert!(MAX_LOG_BYTES > line.weight());
    }
}

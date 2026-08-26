//! État du TUI et transitions.
//!
//! Ce module est **pur** : il ne parle ni au terminal, ni au moteur de
//! conteneurs. Il reçoit des [`Message`] (touche pressée, résultat d'un appel au
//! service) et renvoie éventuellement une [`Command`], c'est-à-dire un effet de
//! bord à exécuter *ailleurs*. C'est ce qui rend la totalité du comportement
//! testable sans terminal et sans Docker.
//!
//! Deux invariants tiennent tout le reste :
//!
//! - la sélection est mémorisée **par identifiant**, jamais par indice : un
//!   rafraîchissement qui réordonne la liste ne déplace pas le curseur ;
//! - aucune action n'est lancée deux fois sur le même conteneur : un conteneur
//!   occupé refuse une nouvelle action jusqu'au retour du moteur.

use std::collections::BTreeSet;

use hormos_core::domain::{ContainerDetails, ContainerSummary};
use hormos_core::error::HormosError;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Action de cycle de vie déclenchable depuis le TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Démarrage.
    Start,
    /// Arrêt.
    Stop,
    /// Redémarrage.
    Restart,
}

impl Verb {
    /// Libellé au participe passé, pour les messages de statut.
    #[must_use]
    pub const fn done(self) -> &'static str {
        match self {
            Self::Start => "démarré",
            Self::Stop => "arrêté",
            Self::Restart => "redémarré",
        }
    }

    /// Libellé à l'infinitif, pour les messages d'attente.
    #[must_use]
    pub const fn pending(self) -> &'static str {
        match self {
            Self::Start => "démarrage",
            Self::Stop => "arrêt",
            Self::Restart => "redémarrage",
        }
    }
}

/// Effet de bord demandé par l'état.
///
/// Le TUI ne déclenche **jamais** d'appel au moteur de lui-même : il en décrit
/// un, et la boucle principale l'exécute hors du rendu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Recharger la liste des conteneurs.
    Refresh {
        /// Inclure les conteneurs arrêtés.
        all: bool,
    },
    /// Charger le détail d'un conteneur.
    Inspect {
        /// Identifiant complet.
        id: String,
    },
    /// Appliquer une action de cycle de vie.
    Act {
        /// Identifiant complet.
        id: String,
        /// Action demandée.
        verb: Verb,
    },
}

/// Événement entrant traité par [`App::update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Touche pressée.
    Key(KeyEvent),
    /// Le terminal a changé de taille (force un rendu).
    Resized,
    /// Résultat d'un rafraîchissement de la liste.
    Containers(Result<Vec<ContainerSummary>, HormosError>),
    /// Résultat d'un `inspect`.
    Details(Result<Box<ContainerDetails>, HormosError>),
    /// Résultat d'une action de cycle de vie.
    Acted {
        /// Conteneur concerné.
        id: String,
        /// Action appliquée.
        verb: Verb,
        /// Issue de l'action.
        outcome: Result<(), HormosError>,
    },
}

/// Gravité d'un message de statut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Information neutre.
    Info,
    /// Échec.
    Error,
}

/// Message affiché en pied d'écran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Texte du message.
    pub text: String,
    /// Gravité associée.
    pub severity: Severity,
}

/// Écran actif.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    /// Liste des conteneurs.
    #[default]
    Browse,
    /// Saisie du filtre ; conserve la valeur d'avant la saisie pour `Échap`.
    Filter {
        /// Filtre en vigueur avant l'ouverture de la barre de saisie.
        previous: String,
    },
    /// Aide.
    Help,
    /// Détail d'un conteneur ; `None` tant que le moteur n'a pas répondu.
    Details(Option<Box<ContainerDetails>>),
}

/// État complet du TUI.
#[derive(Debug)]
pub struct App {
    containers: Vec<ContainerSummary>,
    selected: Option<String>,
    show_all: bool,
    filter: String,
    mode: Mode,
    status: Option<Status>,
    busy: BTreeSet<String>,
    loading: bool,
    failure: Option<String>,
    should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// État initial : liste vide, premier chargement en cours.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            containers: Vec::new(),
            selected: None,
            show_all: false,
            filter: String::new(),
            mode: Mode::Browse,
            status: None,
            busy: BTreeSet::new(),
            loading: true,
            failure: None,
            should_quit: false,
        }
    }

    /// Applique un événement et renvoie l'effet de bord à exécuter, s'il y en a.
    pub fn update(&mut self, message: Message) -> Option<Command> {
        match message {
            Message::Key(key) => self.on_key(key),
            Message::Resized => None,
            Message::Containers(result) => {
                self.on_containers(result);
                None
            }
            Message::Details(result) => {
                self.on_details(result);
                None
            }
            Message::Acted { id, verb, outcome } => self.on_acted(&id, verb, outcome),
        }
    }

    // ---------------------------------------------------------------- lecture

    /// Conteneurs visibles, c'est-à-dire retenus par le filtre courant.
    #[must_use]
    pub fn visible(&self) -> Vec<&ContainerSummary> {
        self.containers
            .iter()
            .filter(|container| matches_filter(container, &self.filter))
            .collect()
    }

    /// Indice du conteneur sélectionné dans la liste visible.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        self.visible()
            .iter()
            .position(|container| container.id == selected)
    }

    /// Conteneur sélectionné.
    #[must_use]
    pub fn selected(&self) -> Option<&ContainerSummary> {
        let selected = self.selected.as_deref()?;
        self.containers
            .iter()
            .find(|container| container.id == selected)
    }

    /// Écran actif.
    #[must_use]
    pub const fn mode(&self) -> &Mode {
        &self.mode
    }

    /// Message de statut courant.
    #[must_use]
    pub const fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// Filtre courant (déjà saisi, hors barre de saisie).
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Indique si les conteneurs arrêtés sont inclus.
    #[must_use]
    pub const fn show_all(&self) -> bool {
        self.show_all
    }

    /// Indique qu'un appel au moteur est en cours.
    #[must_use]
    pub const fn is_loading(&self) -> bool {
        self.loading
    }

    /// Indique qu'une action est en cours sur ce conteneur.
    #[must_use]
    pub fn is_busy(&self, id: &str) -> bool {
        self.busy.contains(id)
    }

    /// Message du dernier échec de rafraîchissement, s'il n'a pas été résorbé.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Indique que la boucle principale doit s'arrêter.
    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    // ------------------------------------------------------------- transitions

    fn on_key(&mut self, key: KeyEvent) -> Option<Command> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return None;
        }
        match &self.mode {
            Mode::Browse => self.on_browse_key(key),
            Mode::Filter { previous } => {
                let previous = previous.clone();
                self.on_filter_key(key, &previous);
                None
            }
            Mode::Help | Mode::Details(_) => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Char('i')
                ) {
                    self.mode = Mode::Browse;
                }
                None
            }
        }
    }

    fn on_browse_key(&mut self, key: KeyEvent) -> Option<Command> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::Home => {
                self.select_at(0);
                None
            }
            KeyCode::End => {
                self.select_at(self.visible().len().saturating_sub(1));
                None
            }
            KeyCode::Char('a') => {
                self.show_all = !self.show_all;
                Some(self.refresh())
            }
            KeyCode::Char('R') => Some(self.refresh()),
            KeyCode::Char('i') => {
                let id = self.selected.clone()?;
                self.mode = Mode::Details(None);
                Some(Command::Inspect { id })
            }
            KeyCode::Char('s') => {
                let verb = if self.selected()?.state.is_running() {
                    Verb::Stop
                } else {
                    Verb::Start
                };
                self.act(verb)
            }
            KeyCode::Char('r') => self.act(Verb::Restart),
            KeyCode::Char('/') => {
                self.mode = Mode::Filter {
                    previous: self.filter.clone(),
                };
                None
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                None
            }
            _ => None,
        }
    }

    fn on_filter_key(&mut self, key: KeyEvent, previous: &str) {
        match key.code {
            KeyCode::Esc => {
                self.filter = previous.to_owned();
                self.mode = Mode::Browse;
                self.reconcile_selection(self.selected_index().unwrap_or(0));
            }
            KeyCode::Enter => self.mode = Mode::Browse,
            KeyCode::Backspace => {
                self.filter.pop();
                self.reconcile_selection(0);
            }
            // Les caractères de contrôle ne sont jamais insérés dans le filtre.
            KeyCode::Char(c) if !c.is_control() => {
                self.filter.push(c);
                self.reconcile_selection(0);
            }
            _ => {}
        }
    }

    fn on_containers(&mut self, result: Result<Vec<ContainerSummary>, HormosError>) {
        self.loading = false;
        match result {
            Ok(containers) => {
                let previous = self.selected_index().unwrap_or(0);
                self.containers = containers;
                self.failure = None;
                self.reconcile_selection(previous);
            }
            Err(error) => {
                // La liste précédente est conservée : mieux vaut des données
                // légèrement datées, explicitement signalées, qu'un écran vide.
                self.failure = Some(error.to_string());
                self.status = Some(Status {
                    text: error.to_string(),
                    severity: Severity::Error,
                });
            }
        }
    }

    fn on_details(&mut self, result: Result<Box<ContainerDetails>, HormosError>) {
        match result {
            Ok(details) => {
                // Un `inspect` qui revient après la fermeture du panneau ne doit
                // pas le rouvrir dans le dos de l'utilisateur.
                if matches!(self.mode, Mode::Details(_)) {
                    self.mode = Mode::Details(Some(details));
                }
            }
            Err(error) => {
                if matches!(self.mode, Mode::Details(_)) {
                    self.mode = Mode::Browse;
                }
                self.status = Some(Status {
                    text: error.to_string(),
                    severity: Severity::Error,
                });
            }
        }
    }

    fn on_acted(
        &mut self,
        id: &str,
        verb: Verb,
        outcome: Result<(), HormosError>,
    ) -> Option<Command> {
        self.busy.remove(id);
        self.status = Some(match outcome {
            Ok(()) => Status {
                text: format!("conteneur {}", verb.done()),
                severity: Severity::Info,
            },
            Err(error) => Status {
                text: error.to_string(),
                severity: Severity::Error,
            },
        });
        // Même après un échec : l'état réel du conteneur a pu changer.
        Some(self.refresh())
    }

    // ---------------------------------------------------------------- interne

    fn refresh(&mut self) -> Command {
        self.loading = true;
        Command::Refresh { all: self.show_all }
    }

    fn act(&mut self, verb: Verb) -> Option<Command> {
        let id = self.selected.clone()?;
        if self.busy.contains(&id) {
            self.status = Some(Status {
                text: "une action est déjà en cours sur ce conteneur".to_owned(),
                severity: Severity::Info,
            });
            return None;
        }
        self.busy.insert(id.clone());
        self.status = Some(Status {
            text: format!("{} en cours…", verb.pending()),
            severity: Severity::Info,
        });
        Some(Command::Act { id, verb })
    }

    fn move_selection(&mut self, delta: isize) {
        let visible = self.visible().len();
        if visible == 0 {
            return;
        }
        let current = self.selected_index().unwrap_or(0);
        let last = visible - 1;
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta.unsigned_abs()).min(last)
        };
        self.select_at(next);
    }

    fn select_at(&mut self, index: usize) {
        self.selected = self
            .visible()
            .get(index)
            .map(|container| container.id.clone());
    }

    /// Rétablit une sélection cohérente après un changement de liste ou de
    /// filtre. La sélection **par identifiant** est conservée si le conteneur
    /// est toujours visible ; sinon le curseur reste à la même position, bornée
    /// à la nouvelle taille de liste.
    fn reconcile_selection(&mut self, previous_index: usize) {
        let visible: Vec<String> = self
            .visible()
            .iter()
            .map(|container| container.id.clone())
            .collect();
        if visible.is_empty() {
            self.selected = None;
            return;
        }
        if let Some(selected) = &self.selected
            && visible.iter().any(|id| id == selected)
        {
            return;
        }
        let index = previous_index.min(visible.len() - 1);
        self.selected = visible.get(index).cloned();
    }
}

/// Filtre local, insensible à la casse, sur le nom, l'image et l'identifiant.
fn matches_filter(container: &ContainerSummary, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_lowercase();
    container.name.to_lowercase().contains(&needle)
        || container.image.to_lowercase().contains(&needle)
        || container.id.to_lowercase().contains(&needle)
}

#[cfg(test)]
mod tests {
    use hormos_core::domain::ContainerState;
    use hormos_core::error::HormosError;

    use super::{
        App, Command, ContainerSummary, KeyCode, KeyEvent, KeyModifiers, Message, Mode, Severity,
        Verb,
    };

    fn summary(id: &str, name: &str, state: ContainerState) -> ContainerSummary {
        ContainerSummary {
            id: id.to_owned(),
            name: name.to_owned(),
            image: "alpine:3.22".to_owned(),
            status: state.as_str().to_owned(),
            state,
            created: Some(1_700_000_000),
        }
    }

    fn key(code: KeyCode) -> Message {
        Message::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn press(app: &mut App, code: KeyCode) -> Option<Command> {
        app.update(key(code))
    }

    fn loaded(containers: Vec<ContainerSummary>) -> Message {
        Message::Containers(Ok(containers))
    }

    fn fixture() -> App {
        let mut app = App::new();
        app.update(loaded(vec![
            summary("id-web", "web", ContainerState::Running),
            summary("id-db", "db", ContainerState::Exited),
            summary("id-cache", "cache", ContainerState::Running),
        ]));
        app
    }

    #[test]
    fn the_first_container_is_selected_after_the_first_load() {
        let app = fixture();
        assert_eq!(app.selected().map(|c| c.name.as_str()), Some("web"));
        assert_eq!(app.selected_index(), Some(0));
        assert!(!app.is_loading());
    }

    #[test]
    fn navigation_stays_within_bounds() {
        let mut app = fixture();
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected_index(), Some(0), "remontée au-dessus du début");

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_index(), Some(2), "descente au-delà de la fin");

        press(&mut app, KeyCode::Home);
        assert_eq!(app.selected_index(), Some(0));
        press(&mut app, KeyCode::End);
        assert_eq!(app.selected_index(), Some(2));
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_index(), Some(1));
    }

    #[test]
    fn the_selection_follows_the_identifier_not_the_row() {
        let mut app = fixture();
        press(&mut app, KeyCode::End);
        assert_eq!(
            app.selected().map(|c| c.id.clone()),
            Some("id-cache".into())
        );

        // La liste revient réordonnée : le curseur doit suivre l'identifiant.
        app.update(loaded(vec![
            summary("id-cache", "cache", ContainerState::Running),
            summary("id-web", "web", ContainerState::Running),
        ]));
        assert_eq!(
            app.selected().map(|c| c.id.clone()),
            Some("id-cache".into())
        );
        assert_eq!(app.selected_index(), Some(0));
    }

    #[test]
    fn a_vanished_identifier_falls_back_to_the_same_position() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected().map(|c| c.name.clone()), Some("db".into()));

        app.update(loaded(vec![
            summary("id-web", "web", ContainerState::Running),
            summary("id-cache", "cache", ContainerState::Running),
        ]));
        assert_eq!(app.selected_index(), Some(1));
        assert_eq!(app.selected().map(|c| c.name.clone()), Some("cache".into()));
    }

    #[test]
    fn an_empty_list_clears_the_selection() {
        let mut app = fixture();
        app.update(loaded(Vec::new()));
        assert_eq!(app.selected(), None);
        assert_eq!(app.selected_index(), None);
        assert!(app.visible().is_empty());
        // Aucune action ne doit partir sans sélection.
        assert_eq!(press(&mut app, KeyCode::Char('s')), None);
        assert_eq!(press(&mut app, KeyCode::Char('i')), None);
    }

    #[test]
    fn toggling_all_asks_for_a_refresh() {
        let mut app = fixture();
        assert!(!app.show_all());
        assert_eq!(
            press(&mut app, KeyCode::Char('a')),
            Some(Command::Refresh { all: true })
        );
        assert!(app.show_all());
        assert!(app.is_loading());
        assert_eq!(
            press(&mut app, KeyCode::Char('a')),
            Some(Command::Refresh { all: false })
        );
        assert!(!app.show_all());
    }

    #[test]
    fn manual_refresh_keeps_the_current_scope() {
        let mut app = fixture();
        assert_eq!(
            press(&mut app, KeyCode::Char('R')),
            Some(Command::Refresh { all: false })
        );
        assert!(app.is_loading());
    }

    #[test]
    fn the_filter_narrows_the_list_and_can_be_cancelled() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('/'));
        assert!(matches!(app.mode(), Mode::Filter { .. }));

        for c in "ca".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.visible().len(), 1);
        assert_eq!(app.selected().map(|c| c.name.clone()), Some("cache".into()));

        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.visible().len(), 1, "« c » ne retient que cache");

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.filter(), "");
        assert_eq!(app.visible().len(), 3);
        assert!(matches!(app.mode(), Mode::Browse));
    }

    #[test]
    fn the_filter_is_case_insensitive_and_matches_image_and_identifier() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('/'));
        for c in "WEB".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.visible().len(), 1);

        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('/'));
        for c in "ALPINE".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.visible().len(), 3);

        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('/'));
        for c in "id-db".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.visible().len(), 1);
    }

    #[test]
    fn a_filter_without_result_clears_the_selection_and_blocks_actions() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('/'));
        for c in "zzz".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert!(app.visible().is_empty());
        assert_eq!(app.selected(), None);
        assert_eq!(press(&mut app, KeyCode::Char('r')), None);
    }

    #[test]
    fn control_characters_never_enter_the_filter() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('\u{1b}'));
        press(&mut app, KeyCode::Char('\u{7}'));
        press(&mut app, KeyCode::Char('w'));
        assert_eq!(app.filter(), "w");
    }

    #[test]
    fn the_action_key_is_contextual() {
        let mut app = fixture();
        // « web » tourne : `s` doit l'arrêter.
        assert_eq!(
            press(&mut app, KeyCode::Char('s')),
            Some(Command::Act {
                id: "id-web".into(),
                verb: Verb::Stop
            })
        );

        let mut app = fixture();
        press(&mut app, KeyCode::Char('j'));
        // « db » est arrêté : `s` doit le démarrer.
        assert_eq!(
            press(&mut app, KeyCode::Char('s')),
            Some(Command::Act {
                id: "id-db".into(),
                verb: Verb::Start
            })
        );
    }

    #[test]
    fn only_one_action_at_a_time_per_container() {
        let mut app = fixture();
        assert!(press(&mut app, KeyCode::Char('r')).is_some());
        assert!(app.is_busy("id-web"));

        assert_eq!(
            press(&mut app, KeyCode::Char('r')),
            None,
            "seconde action acceptée sur un conteneur occupé"
        );
        assert_eq!(press(&mut app, KeyCode::Char('s')), None);

        // Un autre conteneur reste pilotable pendant ce temps.
        press(&mut app, KeyCode::Char('j'));
        assert!(press(&mut app, KeyCode::Char('r')).is_some());
    }

    #[test]
    fn an_action_result_frees_the_container_and_refreshes() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('r'));

        let command = app.update(Message::Acted {
            id: "id-web".into(),
            verb: Verb::Restart,
            outcome: Ok(()),
        });
        assert_eq!(command, Some(Command::Refresh { all: false }));
        assert!(!app.is_busy("id-web"));
        assert_eq!(app.status().map(|s| s.severity), Some(Severity::Info));
    }

    #[test]
    fn a_failed_action_is_reported_and_still_refreshes() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('r'));

        let command = app.update(Message::Acted {
            id: "id-web".into(),
            verb: Verb::Restart,
            outcome: Err(HormosError::Conflict {
                detail: "déjà en cours".into(),
            }),
        });
        assert_eq!(command, Some(Command::Refresh { all: false }));
        assert!(!app.is_busy("id-web"));
        let status = app.status().cloned();
        assert_eq!(status.as_ref().map(|s| s.severity), Some(Severity::Error));
        assert!(status.is_some_and(|s| s.text.contains("déjà en cours")));
    }

    #[test]
    fn inspect_is_on_demand_and_closes_on_escape() {
        let mut app = fixture();
        assert_eq!(
            press(&mut app, KeyCode::Char('i')),
            Some(Command::Inspect {
                id: "id-web".into()
            })
        );
        assert!(matches!(app.mode(), Mode::Details(None)));

        app.update(Message::Details(Ok(Box::new(details("web")))));
        assert!(matches!(app.mode(), Mode::Details(Some(_))));

        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode(), Mode::Browse));
        assert!(
            !app.should_quit(),
            "Échap a fermé le TUI au lieu du panneau"
        );
    }

    #[test]
    fn a_late_inspect_result_never_reopens_a_closed_panel() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('i'));
        press(&mut app, KeyCode::Esc);

        app.update(Message::Details(Ok(Box::new(details("web")))));
        assert!(matches!(app.mode(), Mode::Browse));
    }

    #[test]
    fn a_failed_inspect_closes_the_panel_and_reports() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('i'));
        app.update(Message::Details(Err(HormosError::ContainerNotFound {
            reference: "web".into(),
        })));
        assert!(matches!(app.mode(), Mode::Browse));
        assert_eq!(app.status().map(|s| s.severity), Some(Severity::Error));
    }

    #[test]
    fn help_opens_and_closes_without_quitting() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('?'));
        assert!(matches!(app.mode(), Mode::Help));
        press(&mut app, KeyCode::Char('j'));
        assert!(
            matches!(app.mode(), Mode::Help),
            "l'aide s'est fermée seule"
        );
        press(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.mode(), Mode::Browse));
        assert!(!app.should_quit(), "« q » a fermé le TUI au lieu de l'aide");
    }

    #[test]
    fn a_refresh_failure_keeps_the_previous_list() {
        let mut app = fixture();
        app.update(Message::Containers(Err(HormosError::DaemonUnavailable {
            detail: "/var/run/docker.sock".into(),
        })));
        assert_eq!(app.visible().len(), 3, "liste vidée par un échec");
        assert!(app.failure().is_some_and(|f| f.contains("docker.sock")));
        assert!(!app.is_loading());

        // Une reprise réussie efface l'écran d'erreur.
        app.update(loaded(vec![summary(
            "id-web",
            "web",
            ContainerState::Running,
        )]));
        assert_eq!(app.failure(), None);
    }

    #[test]
    fn quitting_is_explicit() {
        let mut app = fixture();
        assert!(!app.should_quit());
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit());

        let mut app = fixture();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.should_quit());
    }

    #[test]
    fn a_resize_changes_nothing_but_triggers_a_redraw() {
        let mut app = fixture();
        let before = app.selected().cloned();
        assert_eq!(app.update(Message::Resized), None);
        assert_eq!(app.selected().cloned(), before);
    }

    fn details(name: &str) -> hormos_core::domain::ContainerDetails {
        hormos_core::domain::ContainerDetails {
            id: format!("id-{name}"),
            name: name.to_owned(),
            image: "alpine:3.22".to_owned(),
            state: ContainerState::Running,
            status: Some("Up 2 hours".to_owned()),
            created: Some("2026-01-01T00:00:00Z".to_owned()),
            hostname: Some("box".to_owned()),
            restart_count: Some(0),
        }
    }
}

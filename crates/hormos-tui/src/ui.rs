//! Rendu de l'écran Conteneurs.
//!
//! Ce module ne parle **jamais** au moteur : il ne lit que l'[`App`]. Toute
//! chaîne provenant du moteur (nom, image, état, statut) est assainie ici, à la
//! frontière d'affichage, par [`hormos_core::display`] : un nom de conteneur est
//! choisi par celui qui a créé le conteneur, donc non fiable. Un terminal, plus
//! encore qu'une sortie standard, est pilotable par séquences d'échappement.

use hormos_core::display::{sanitize, sanitize_truncated};
use hormos_core::domain::{ContainerDetails, ContainerSummary};
use hormos_core::logs::LogSource;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};

use crate::app::{App, Mode, Severity};
use crate::stream::StreamState;

/// Largeur minimale exploitable.
pub const MIN_WIDTH: u16 = 60;

/// Hauteur minimale exploitable.
pub const MIN_HEIGHT: u16 = 12;

/// Largeur d'affichage d'un identifiant, alignée sur la CLI et sur Docker.
const SHORT_ID_LEN: usize = 12;

/// Largeur maximale d'une colonne de texte libre.
const COLUMN_MAX: usize = 40;

/// Largeur maximale d'un nom d'action d'événement.
const EVENT_ACTION_MAX: usize = 12;

/// Aide-mémoire affiché en pied d'écran.
const HINTS: &str = "q quitter · ↑↓/jk naviguer · a tous · R rafraîchir · / filtrer · i détail · l journal · e événements · s start/stop · r restart · ? aide";

/// Aide-mémoire affiché sous un flux.
const STREAM_HINTS: &str =
    "Échap/q retour · ↑↓/jk défiler · PgPréc/PgSuiv page · Début/Fin · R reconnecter";

/// Dessine l'image complète.
pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    if matches!(app.mode(), Mode::Logs { .. } | Mode::Events) {
        render_stream(app, frame, area);
        return;
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(app, frame, header);
    if app.visible().is_empty()
        && let Some(failure) = app.failure()
    {
        render_failure(frame, body, failure);
    } else {
        render_table(app, frame, body);
    }
    render_footer(app, frame, footer);

    match app.mode() {
        Mode::Help => render_help(frame, area),
        Mode::Details(details) => render_details(frame, area, details.as_deref()),
        Mode::Browse | Mode::Filter { .. } | Mode::Logs { .. } | Mode::Events => {}
    }
}

/// Hauteur utile du panneau de flux pour une surface donnée.
///
/// Elle est calculée ici, à côté de la mise en page, plutôt que devinée par
/// l'état : c'est la disposition qui décide, et une seule formule évite qu'un
/// « page suivante » saute des lignes.
#[must_use]
pub const fn stream_viewport(area: Rect) -> usize {
    // En-tête, pied d'écran, et les deux bordures du cadre.
    (area.height.saturating_sub(4)) as usize
}

/// Dessine un flux en plein écran : journal ou événements.
fn render_stream(app: &App, frame: &mut Frame, area: Rect) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let viewport = stream_viewport(area);
    match app.mode() {
        Mode::Logs { name, .. } => {
            render_stream_header(app, frame, header, &format!("journal · {}", sanitize(name)));
            render_log_lines(app, frame, body, viewport);
        }
        _ => {
            render_stream_header(app, frame, header, "événements du moteur");
            render_events(app, frame, body, viewport);
        }
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            STREAM_HINTS,
            Style::default().fg(Color::DarkGray),
        )),
        footer,
    );
}

fn render_stream_header(app: &App, frame: &mut Frame, area: Rect, title: &str) {
    let (kept, dropped, follows) = match app.mode() {
        Mode::Events => (
            app.events().len(),
            app.events().dropped(),
            app.events().follows(),
        ),
        _ => (app.logs().len(), app.logs().dropped(), app.logs().follows()),
    };

    let mut spans = vec![
        Span::styled(
            "Hormos",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" · {title} · {kept}")),
    ];
    if dropped > 0 {
        spans.push(Span::styled(
            format!(" · {dropped} perdus"),
            Style::default().fg(Color::Yellow),
        ));
    }
    spans.push(match app.stream() {
        StreamState::Idle => Span::raw(String::new()),
        StreamState::Connecting => {
            Span::styled(" · connexion…", Style::default().fg(Color::Yellow))
        }
        StreamState::Live if follows => {
            Span::styled(" · en direct", Style::default().fg(Color::Green))
        }
        StreamState::Live => Span::styled(
            " · en pause (Fin reprend)",
            Style::default().fg(Color::Yellow),
        ),
        StreamState::Ended(None) => {
            Span::styled(" · terminé", Style::default().fg(Color::DarkGray))
        }
        StreamState::Ended(Some(error)) => Span::styled(
            format!(
                " · interrompu : {}",
                sanitize_truncated(error, COLUMN_MAX * 2)
            ),
            Style::default().fg(Color::Red),
        ),
    });
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_log_lines(app: &App, frame: &mut Frame, area: Rect, viewport: usize) {
    if app.logs().is_empty() {
        render_empty_stream(frame, area, "Aucune ligne pour l'instant.");
        return;
    }
    // Les lignes ont déjà été décodées et neutralisées par le découpeur ; elles
    // repassent tout de même par l'assainissement d'affichage, qui est la seule
    // frontière dont dépend la sûreté du terminal.
    let lines: Vec<Line> = app
        .logs()
        .visible(viewport)
        .map(|line| {
            let style = if line.source == LogSource::Stderr {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            Line::styled(sanitize(&line.text), style)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(container_block()),
        area,
    );
}

fn render_events(app: &App, frame: &mut Frame, area: Rect, viewport: usize) {
    if app.events().is_empty() {
        render_empty_stream(frame, area, "Aucun événement pour l'instant.");
        return;
    }
    let lines: Vec<Line> = app
        .events()
        .visible(viewport)
        .map(|event| {
            Line::from(vec![
                Span::styled(
                    format!("{:<21}", sanitize(&event.formatted_time())),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{:<10}", event.kind.as_str()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(format!(
                    "{:<12}",
                    sanitize_truncated(&event.action, EVENT_ACTION_MAX)
                )),
                Span::raw(format!(
                    "{:<14}",
                    event
                        .short_id(SHORT_ID_LEN)
                        .map_or_else(|| "-".to_owned(), |id| sanitize(&id))
                )),
                Span::raw(
                    event
                        .actor_name
                        .as_deref()
                        .map_or_else(String::new, |name| sanitize_truncated(name, COLUMN_MAX)),
                ),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(container_block()),
        area,
    );
}

fn render_empty_stream(frame: &mut Frame, area: Rect, message: &str) {
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true })
            .centered()
            .block(container_block()),
        area,
    );
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let scope = if app.show_all() {
        "tous"
    } else {
        "en exécution"
    };
    let mut spans = vec![
        Span::styled(
            "Hormos",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" · conteneurs ({scope}) · {}", app.visible().len())),
    ];
    if !app.filter().is_empty() {
        // Le filtre est saisi par l'utilisateur, mais il est réaffiché tel quel :
        // il passe donc par le même assainissement que le reste.
        spans.push(Span::raw(format!(
            " · filtre « {} »",
            sanitize_truncated(app.filter(), COLUMN_MAX)
        )));
    }
    if app.is_loading() {
        spans.push(Span::styled(
            " · chargement…",
            Style::default().fg(Color::Yellow),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    if let Mode::Filter { .. } = app.mode() {
        let line = Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Cyan)),
            Span::raw(sanitize_truncated(app.filter(), COLUMN_MAX)),
            Span::styled("▏", Style::default().fg(Color::Cyan)),
            Span::raw("  (Entrée valide · Échap annule)"),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let paragraph = app.status().map_or_else(
        || Paragraph::new(Span::styled(HINTS, Style::default().fg(Color::DarkGray))),
        |status| {
            let color = match status.severity {
                Severity::Info => Color::Green,
                Severity::Error => Color::Red,
            };
            Paragraph::new(Span::styled(
                sanitize(&status.text),
                Style::default().fg(color),
            ))
        },
    );
    frame.render_widget(paragraph, area);
}

fn render_table(app: &App, frame: &mut Frame, area: Rect) {
    let containers = app.visible();
    if containers.is_empty() {
        let message = if app.filter().is_empty() {
            "Aucun conteneur. « a » inclut les conteneurs arrêtés, « R » rafraîchit."
        } else {
            "Aucun conteneur ne correspond au filtre. « / » puis Échap l'efface."
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(Color::DarkGray))
                .wrap(Wrap { trim: true })
                .centered()
                .block(container_block()),
            area,
        );
        return;
    }

    let header = Row::new(["ID", "NOM", "IMAGE", "ÉTAT", "STATUT"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = containers
        .iter()
        .map(|container| row(app, container))
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(SHORT_ID_LEN as u16),
            Constraint::Length(20),
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(container_block())
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
    .highlight_symbol("▸ ");

    let mut state = TableState::default().with_selected(app.selected_index());
    frame.render_stateful_widget(table, area, &mut state);
}

fn row<'a>(app: &App, container: &'a ContainerSummary) -> Row<'a> {
    let busy = app.is_busy(&container.id);
    let status = if busy {
        Cell::from("action en cours…").style(Style::default().fg(Color::Yellow))
    } else {
        Cell::from(sanitize_truncated(&container.status, COLUMN_MAX))
    };
    let state_color = if container.state.is_running() {
        Color::Green
    } else {
        Color::DarkGray
    };

    Row::new(vec![
        Cell::from(short_id(&container.id)),
        Cell::from(sanitize_truncated(&container.name, COLUMN_MAX)),
        Cell::from(sanitize_truncated(&container.image, COLUMN_MAX)),
        Cell::from(sanitize_truncated(container.state.as_str(), COLUMN_MAX))
            .style(Style::default().fg(state_color)),
        status,
    ])
}

fn render_failure(frame: &mut Frame, area: Rect, failure: &str) {
    let text = Text::from(vec![
        Line::styled(
            "Le moteur de conteneurs n'a pas répondu",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw(sanitize(failure)),
        Line::raw(""),
        Line::styled(
            "« R » réessaie · « q » quitte",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .centered()
            .block(container_block()),
        area,
    );
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let text = Text::from(vec![
        Line::raw("Terminal trop petit"),
        Line::raw(format!("minimum {MIN_WIDTH}×{MIN_HEIGHT}")),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(Color::Yellow))
            .wrap(Wrap { trim: true })
            .centered(),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let lines = [
        ("↑ ↓ / k j", "déplacer la sélection"),
        ("Début / Fin", "premier / dernier conteneur"),
        ("a", "inclure ou non les conteneurs arrêtés"),
        ("R", "rafraîchir la liste"),
        ("/", "filtrer sur le nom, l'image ou l'identifiant"),
        ("i", "afficher le détail du conteneur"),
        ("l", "afficher le journal du conteneur"),
        ("e", "afficher les événements du moteur"),
        ("s", "démarrer ou arrêter selon l'état"),
        ("r", "redémarrer"),
        ("?", "afficher ou masquer cette aide"),
        ("q · Échap · Ctrl+C", "quitter"),
    ];
    let text = Text::from(
        lines
            .iter()
            .map(|(keys, description)| {
                Line::from(vec![
                    Span::styled(
                        format!("{keys:<20}"),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(*description),
                ])
            })
            .collect::<Vec<_>>(),
    );
    popup(frame, area, "Aide", Paragraph::new(text), 64, 16);
}

fn render_details(frame: &mut Frame, area: Rect, details: Option<&ContainerDetails>) {
    let text = details.map_or_else(
        || Text::from("Chargement…"),
        |details| {
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
            Text::from(
                rows.iter()
                    .map(|(label, value)| {
                        Line::from(vec![
                            Span::styled(format!("{label:<14}"), Style::default().fg(Color::Cyan)),
                            Span::raw(value.clone()),
                        ])
                    })
                    .collect::<Vec<_>>(),
            )
        },
    );
    popup(
        frame,
        area,
        "Détail · Échap ferme",
        Paragraph::new(text).wrap(Wrap { trim: true }),
        72,
        12,
    );
}

/// Dessine un panneau centré, en effaçant ce qu'il recouvre.
fn popup(frame: &mut Frame, area: Rect, title: &str, content: Paragraph, width: u16, height: u16) {
    let area = centered(area, width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        content.block(container_block().title(format!(" {title} "))),
        area,
    );
}

/// Rectangle centré, borné à la surface disponible.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn container_block() -> Block<'static> {
    Block::bordered().border_type(BorderType::Rounded)
}

fn short_id(id: &str) -> String {
    sanitize(id).chars().take(SHORT_ID_LEN).collect()
}

fn optional(value: Option<&str>) -> String {
    value.map_or_else(|| "<inconnu>".to_owned(), sanitize)
}

fn count(value: Option<i64>) -> String {
    value.map_or_else(|| "<inconnu>".to_owned(), |v| v.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use hormos_core::domain::{ContainerDetails, ContainerState, ContainerSummary};
    use hormos_core::error::HormosError;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use hormos_core::events::{ResourceKind, RuntimeEvent};
    use hormos_core::logs::LogSource;
    use ratatui::layout::Rect;

    use super::{MIN_HEIGHT, MIN_WIDTH, render, stream_viewport};
    use crate::app::{App, Message};
    use crate::stream::LogLine;

    fn summary(id: &str, name: &str, image: &str, state: ContainerState) -> ContainerSummary {
        ContainerSummary {
            id: id.to_owned(),
            name: name.to_owned(),
            image: image.to_owned(),
            status: "Up 2 hours".to_owned(),
            state,
            created: Some(1_700_000_000),
        }
    }

    fn fixture() -> App {
        let mut app = App::new();
        app.update(Message::Containers(Ok(vec![
            summary(
                "0123456789abcdef0123456789abcdef",
                "web",
                "alpine:3.22",
                ContainerState::Running,
            ),
            summary(
                "fedcba9876543210",
                "db",
                "postgres:17",
                ContainerState::Exited,
            ),
        ])));
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    /// Rend l'application dans un terminal de test et renvoie l'écran obtenu.
    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(app, frame)).unwrap();
        flatten(terminal.backend().buffer())
    }

    fn flatten(buffer: &Buffer) -> String {
        let area = buffer.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_list_renders_at_the_usual_sizes() {
        for (width, height) in [(120, 30), (80, 24)] {
            let rendered = screen(&fixture(), width, height);
            assert!(
                rendered.contains("Hormos"),
                "{width}×{height} : sans en-tête"
            );
            assert!(
                rendered.contains("web"),
                "{width}×{height} : sans conteneur"
            );
            assert!(
                rendered.contains("db"),
                "{width}×{height} : conteneur manquant"
            );
            assert!(
                rendered.contains("0123456789ab"),
                "{width}×{height} : identifiant absent"
            );
            assert!(
                !rendered.contains("0123456789abc"),
                "{width}×{height} : identifiant non tronqué"
            );
            assert!(
                rendered.contains('▸'),
                "{width}×{height} : sélection invisible"
            );
        }
    }

    #[test]
    fn a_cramped_terminal_asks_for_more_room() {
        let rendered = screen(&fixture(), 40, 10);
        assert!(rendered.contains("trop petit"));
        assert!(
            !rendered.contains("web"),
            "la liste est dessinée en désordre"
        );
    }

    #[test]
    fn the_minimum_size_is_exactly_usable() {
        let rendered = screen(&fixture(), MIN_WIDTH, MIN_HEIGHT);
        assert!(!rendered.contains("trop petit"));
        assert!(rendered.contains("web"));
    }

    #[test]
    fn hostile_strings_cannot_drive_the_terminal() {
        let mut app = App::new();
        app.update(Message::Containers(Ok(vec![summary(
            "id\u{1b}[2K",
            "web\u{1b}[31m\u{7}",
            "alp\rine",
            ContainerState::from_engine("run\u{9b}ning"),
        )])));
        let rendered = screen(&app, 120, 30);
        assert!(!rendered.contains('\u{1b}'), "séquence ANSI conservée");
        assert!(!rendered.contains('\u{7}'), "BEL conservé");
        assert!(!rendered.contains('\u{9b}'), "CSI 8 bits conservé");
    }

    #[test]
    fn the_help_panel_covers_the_list() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('?'));
        let rendered = screen(&app, 120, 30);
        assert!(rendered.contains("Aide"));
        assert!(rendered.contains("redémarrer"));
    }

    #[test]
    fn the_details_panel_shows_loading_then_values() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('i'));
        assert!(screen(&app, 120, 30).contains("Chargement…"));

        app.update(Message::Details(Ok(Box::new(ContainerDetails {
            id: "0123456789abcdef".to_owned(),
            name: "web".to_owned(),
            image: "alpine:3.22".to_owned(),
            state: ContainerState::Running,
            status: Some("Up 2 hours".to_owned()),
            created: Some("2026-01-01T00:00:00Z".to_owned()),
            hostname: Some("box".to_owned()),
            restart_count: Some(0),
        }))));
        let rendered = screen(&app, 120, 30);
        assert!(rendered.contains("Nom d'hôte"));
        assert!(rendered.contains("box"));
    }

    #[test]
    fn an_unreachable_daemon_shows_a_retry_screen() {
        let mut app = App::new();
        app.update(Message::Containers(Err(HormosError::DaemonUnavailable {
            detail: "/var/run/docker.sock".into(),
        })));
        let rendered = screen(&app, 120, 30);
        assert!(rendered.contains("n'a pas répondu"));
        assert!(rendered.contains("réessaie"));
    }

    #[test]
    fn an_empty_list_explains_what_to_do() {
        let mut app = App::new();
        app.update(Message::Containers(Ok(Vec::new())));
        assert!(screen(&app, 120, 30).contains("Aucun conteneur."));
    }

    #[test]
    fn a_filter_without_result_explains_how_to_clear_it() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('z'));
        let rendered = screen(&app, 120, 30);
        assert!(rendered.contains("correspond au filtre"));
        assert!(rendered.contains("Échap annule"), "barre de saisie absente");
    }

    #[test]
    fn a_busy_container_is_visible_in_the_list() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('r'));
        assert!(screen(&app, 120, 30).contains("action en cours…"));
    }

    // ------------------------------------------------------------------ flux

    /// Ouvre le journal du premier conteneur et y verse des lignes.
    fn with_logs(lines: Vec<LogLine>) -> App {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('l'));
        app.update(Message::Logs {
            generation: 1,
            lines,
        });
        app
    }

    fn stdout(text: &str) -> LogLine {
        LogLine::new(LogSource::Stdout, text.to_owned())
    }

    #[test]
    fn the_log_screen_shows_the_end_of_the_stream() {
        let app = with_logs((0..100).map(|n| stdout(&format!("ligne {n}"))).collect());
        let screen = screen(&app, 80, 12);

        assert!(screen.contains("journal · web"), "{screen}");
        assert!(screen.contains("en direct"), "{screen}");
        assert!(screen.contains("ligne 99"), "{screen}");
        assert!(!screen.contains("ligne 0 "), "le début est encore affiché");
    }

    #[test]
    fn an_empty_log_says_so_rather_than_showing_nothing() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('l'));
        let screen = screen(&app, 80, 12);

        assert!(screen.contains("Aucune ligne"), "{screen}");
        assert!(screen.contains("connexion"), "{screen}");
    }

    #[test]
    fn hostile_log_lines_cannot_drive_the_terminal() {
        // Le découpeur a déjà neutralisé ces octets ; la ligne est fabriquée à
        // la main pour éprouver la frontière d'affichage elle-même.
        let app = with_logs(vec![stdout("\u{1b}[2Jefface\u{7}\u{9b}31m")]);
        let screen = screen(&app, 80, 12);

        assert!(
            !screen.contains('\u{1b}'),
            "séquence ANSI rendue telle quelle"
        );
        assert!(!screen.contains('\u{7}'), "sonnerie rendue telle quelle");
        assert!(!screen.contains('\u{9b}'), "CSI 8 bits rendu tel quel");
        assert!(screen.contains("efface"), "{screen}");
    }

    #[test]
    fn a_paused_log_says_that_it_is_no_longer_following() {
        let mut app = with_logs((0..100).map(|n| stdout(&format!("ligne {n}"))).collect());
        press(&mut app, KeyCode::PageUp);
        let screen = screen(&app, 80, 12);

        assert!(screen.contains("en pause"), "{screen}");
    }

    #[test]
    fn an_interrupted_stream_shows_the_reason_and_keeps_its_lines() {
        let mut app = with_logs(vec![stdout("dernière ligne")]);
        app.update(Message::StreamEnded {
            generation: 1,
            outcome: Err(HormosError::runtime("le moteur a coupé")),
        });
        let screen = screen(&app, 80, 12);

        assert!(screen.contains("interrompu"), "{screen}");
        assert!(screen.contains("dernière ligne"), "{screen}");
    }

    #[test]
    fn dropped_lines_are_announced() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('l'));
        // Deux fois la borne du tampon : la moitié la plus ancienne est perdue.
        app.update(Message::Logs {
            generation: 1,
            lines: (0..(crate::stream::MAX_LOG_LINES * 2))
                .map(|n| stdout(&format!("ligne {n}")))
                .collect(),
        });
        let screen = screen(&app, 80, 12);

        assert!(screen.contains("perdus"), "{screen}");
    }

    #[test]
    fn the_event_screen_lists_what_the_engine_reports() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('e'));
        app.update(Message::Event {
            generation: 1,
            event: Box::new(RuntimeEvent {
                timestamp: Some(1_700_000_000),
                kind: ResourceKind::Container,
                action: "start".to_owned(),
                actor_id: Some("0123456789abcdef".to_owned()),
                actor_name: Some("web\u{1b}[2K".to_owned()),
            }),
        });
        let screen = screen(&app, 80, 12);

        assert!(screen.contains("événements du moteur"), "{screen}");
        assert!(screen.contains("container"), "{screen}");
        assert!(screen.contains("start"), "{screen}");
        assert!(screen.contains("0123456789ab"), "{screen}");
        assert!(!screen.contains('\u{1b}'), "nom d'acteur non assaini");
    }

    #[test]
    fn the_viewport_matches_the_rows_actually_drawn() {
        // Si les deux divergeaient, « page suivante » sauterait des lignes.
        let height = 20;
        let app = with_logs((0..100).map(|n| stdout(&format!("ligne {n}"))).collect());
        let viewport = stream_viewport(Rect::new(0, 0, 80, height));
        let screen = screen(&app, 80, height);

        let drawn = (0..100)
            .filter(|n| screen.contains(&format!("ligne {n} ")))
            .count();
        assert_eq!(drawn, viewport, "{screen}");
    }
}

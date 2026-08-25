//! Rendu de l'écran Conteneurs.
//!
//! Ce module ne parle **jamais** au moteur : il ne lit que l'[`App`]. Toute
//! chaîne provenant du moteur (nom, image, état, statut) est assainie ici, à la
//! frontière d'affichage, par [`hormos_core::display`] : un nom de conteneur est
//! choisi par celui qui a créé le conteneur, donc non fiable. Un terminal, plus
//! encore qu'une sortie standard, est pilotable par séquences d'échappement.

use hormos_core::display::{sanitize, sanitize_truncated};
use hormos_core::domain::{ContainerDetails, ContainerSummary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};

use crate::app::{App, Mode, Severity};

/// Largeur minimale exploitable.
pub const MIN_WIDTH: u16 = 60;

/// Hauteur minimale exploitable.
pub const MIN_HEIGHT: u16 = 12;

/// Largeur d'affichage d'un identifiant, alignée sur la CLI et sur Docker.
const SHORT_ID_LEN: usize = 12;

/// Largeur maximale d'une colonne de texte libre.
const COLUMN_MAX: usize = 40;

/// Aide-mémoire affiché en pied d'écran.
const HINTS: &str = "q quitter · ↑↓/jk naviguer · a tous · R rafraîchir · / filtrer · i détail · s start/stop · r restart · ? aide";

/// Dessine l'image complète.
pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
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
        Mode::Browse | Mode::Filter { .. } => {}
    }
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
    popup(frame, area, "Aide", Paragraph::new(text), 64, 14);
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

    use super::{MIN_HEIGHT, MIN_WIDTH, render};
    use crate::app::{App, Message};

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
}

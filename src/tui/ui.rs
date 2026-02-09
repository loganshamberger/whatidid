use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, Focus, Mode};

/// Render the entire UI into a ratatui frame.
pub fn draw(f: &mut Frame, app: &App) {
    // Top-level vertical split: main area + status bar (+ search bar if in search mode)
    let bottom_height = if app.mode == Mode::Search { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(bottom_height),
        ])
        .split(f.area());

    let main_area = chunks[0];
    let bottom_area = chunks[1];

    // Horizontal split: left pane (30%) and right pane (70%)
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(main_area);

    draw_left_pane(f, app, panes[0]);
    draw_right_pane(f, app, panes[1]);
    draw_bottom(f, app, bottom_area);
}

fn draw_left_pane(f: &mut Frame, app: &App, area: Rect) {
    let title = app.left_pane_title();
    let border_style = if app.focus == Focus::List {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let items: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let text = item.display_text();
            let style = if i == app.cursor {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_right_pane(f: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Focus::Content {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title("Content");

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.content_lines.is_empty() {
        return;
    }

    let lines: Vec<Line> = app
        .content_lines
        .iter()
        .skip(app.content_scroll as usize)
        .map(|l| Line::from(l.as_str()))
        .collect();

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

fn draw_bottom(f: &mut Frame, app: &App, area: Rect) {
    if app.mode == Mode::Search {
        // Split bottom area into hint line and search input
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        let hint = Paragraph::new(Line::from(Span::styled(
            app.status_hint(),
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint, chunks[0]);

        let search_line = format!("/{}", app.search_input);
        let search = Paragraph::new(Line::from(Span::styled(
            search_line,
            Style::default().fg(Color::Yellow),
        )));
        f.render_widget(search, chunks[1]);

        // Position cursor at end of search input
        f.set_cursor_position((
            chunks[1].x + 1 + app.search_input.len() as u16,
            chunks[1].y,
        ));
    } else {
        let hint = Paragraph::new(Line::from(Span::styled(
            app.status_hint(),
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint, area);
    }
}

use crate::app::{App, Line as ChatLine};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // chat history
            Constraint::Length(3), // input
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_history(f, app, chunks[1]);
    draw_input(f, app, chunks[2]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.processing {
        Span::styled(
            " thinking... ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "  ready  ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    };

    let line = Line::from(vec![
        Span::styled(
            " Elasticsearch AI Agent ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("· {} MCP tool(s) ", app.tool_count)),
        status,
    ]);

    let header = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn draw_history(f: &mut Frame, app: &mut App, area: Rect) {
    let inner_width = area.width.saturating_sub(2).max(1);

    let mut lines: Vec<Line> = Vec::new();
    for entry in &app.transcript {
        match entry {
            ChatLine::User(text) => {
                lines.push(Line::from(vec![Span::styled(
                    "you  ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]));
                for l in text.lines() {
                    lines.push(Line::from(format!("  {l}")));
                }
            }
            ChatLine::Assistant(text) => {
                lines.push(Line::from(vec![Span::styled(
                    "agent",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )]));
                for l in text.lines() {
                    lines.push(Line::from(format!("  {l}")));
                }
            }
            ChatLine::Tool(text) => {
                lines.push(Line::from(Span::styled(
                    format!("  {text}"),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            ChatLine::Error(text) => {
                lines.push(Line::from(Span::styled(
                    format!("  error: {text}"),
                    Style::default().fg(Color::Red),
                )));
            }
            ChatLine::System(text) => {
                lines.push(Line::from(Span::styled(
                    format!("  {text}"),
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::DIM),
                )));
            }
        }
        lines.push(Line::default()); // blank separator
    }

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Conversation "),
        )
        .wrap(Wrap { trim: false });

    // Clamp scroll so we never scroll past the bottom, and auto-scroll to
    // the bottom (`scroll == u16::MAX`, set whenever new content arrives).
    let total = paragraph.line_count(inner_width) as u16;
    let viewport = area.height.saturating_sub(2);
    let max_scroll = total.saturating_sub(viewport);
    if app.scroll > max_scroll {
        app.scroll = max_scroll;
    }

    let paragraph = paragraph.scroll((app.scroll, 0));
    f.render_widget(paragraph, area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.processing {
        " Message (waiting for agent...) "
    } else {
        " Message (Enter to send, Esc to quit) "
    };

    let mut content = app.input.clone();
    content.push('▏'); // cursor

    let input = Paragraph::new(content)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(input, area);
}

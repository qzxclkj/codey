mod agent;
mod app;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok(); // loads .env from the current directory into env vars

    let mut terminal = setup_terminal()?;

    let result = run(&mut terminal).await;

    restore_terminal(&mut terminal)?;

    if let Err(e) = &result {
        eprintln!("error: {e:#}");
    }
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    // This performs the MCP handshake + tools/list, so the first frame
    // already shows which MCP tools are available.
    let mut app = App::new().await?;

    let mut events = EventStream::new();

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        app.handle_key(key.code);
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                    _ => {}
                }
            }
            Some(event) = app.agent_rx.recv() => {
                app.handle_agent_event(event);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

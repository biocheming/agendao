use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn enter() -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste,
    )
}

pub fn init() -> io::Result<Tui> {
    enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    Terminal::new(backend)
}

pub fn resume() -> io::Result<()> {
    enter()
}

pub fn restore() -> io::Result<()> {
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

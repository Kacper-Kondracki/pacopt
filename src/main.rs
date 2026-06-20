use std::io::{self, BufWriter};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut terminal = init_terminal()?;
    let result = pacopt::run_app(&mut terminal);
    restore_terminal()?;

    let selected = result?;
    println!("{}", selected.join(" "));

    Ok(())
}

fn init_terminal() -> io::Result<Terminal<CrosstermBackend<BufWriter<io::Stderr>>>> {
    enable_raw_mode()?;
    execute!(io::stderr(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(BufWriter::new(io::stderr()));
    Terminal::new(backend)
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stderr(), LeaveAlternateScreen)?;

    Ok(())
}

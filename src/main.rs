fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut terminal = ratatui::init();
    let result = pacopt::run_app(&mut terminal);
    ratatui::restore();

    result
}

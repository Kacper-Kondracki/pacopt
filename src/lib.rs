//! Terminal UI for reviewing missing optional dependencies on pacman systems.
//!
//! The library exposes the application state type and data models so tests or
//! embedding code can drive the UI without going through the binary entrypoint.

mod app;
mod model;
mod pacman;
mod runtime;
mod ui;

use std::io;

use eyre::Result;
use ratatui::Terminal;
use ratatui::backend::Backend;

pub use app::App;
pub use model::{
    InstalledPackage, MissingOptionalDep, OptionalDep, OptionalDepRequester, PackageInfo,
};
pub use runtime::Msg;

/// Runs the terminal application and returns the selected missing optional dependency names.
///
/// The caller owns terminal setup and teardown. The returned names are sorted
/// because selections are stored in a set.
pub fn run_app<B>(terminal: &mut Terminal<B>) -> Result<Vec<String>>
where
    B: Backend<Error = io::Error>,
{
    App::load()?.run(terminal)
}

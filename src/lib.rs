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

pub fn run_app<B>(terminal: &mut Terminal<B>) -> Result<Vec<String>>
where
    B: Backend<Error = io::Error>,
{
    App::load()?.run(terminal)
}

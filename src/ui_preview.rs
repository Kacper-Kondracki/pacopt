//! Preview fixtures for the terminal UI.
//!
//! This module is public only so the snapshot update example can reuse the same
//! renderer as the tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::app::{App, AppView};
use crate::model::{
    InstalledPackage, MissingOptionalDep, OptionalDep, OptionalDepRequester, PackageInfo,
};

const PREVIEW_WIDTH: u16 = 120;
const PREVIEW_HEIGHT: u16 = 20;

pub const PACKAGE_VIEW_SNAPSHOT_PATH: &str = "tests/fixtures/ui/package_view.txt";
pub const MISSING_OPTIONAL_DEPS_VIEW_SNAPSHOT_PATH: &str =
    "tests/fixtures/ui/missing_optional_deps_view.txt";

pub struct UiPreviewSnapshot {
    pub path: &'static str,
    pub contents: String,
}

pub fn snapshots() -> Vec<UiPreviewSnapshot> {
    vec![
        UiPreviewSnapshot {
            path: PACKAGE_VIEW_SNAPSHOT_PATH,
            contents: package_view(),
        },
        UiPreviewSnapshot {
            path: MISSING_OPTIONAL_DEPS_VIEW_SNAPSHOT_PATH,
            contents: missing_optional_deps_view(),
        },
    ]
}

pub fn package_view() -> String {
    let mut package_app = preview_app();

    render_app(&mut package_app, PREVIEW_WIDTH, PREVIEW_HEIGHT)
}

pub fn missing_optional_deps_view() -> String {
    let mut missing_deps_app = preview_app();
    missing_deps_app.active_view = AppView::MissingOptionalDeps;
    missing_deps_app
        .checked_missing_optional_deps
        .insert("ripgrep-all".to_owned());
    missing_deps_app
        .missing_optional_dep_list_state
        .select(Some(1));

    render_app(&mut missing_deps_app, PREVIEW_WIDTH, PREVIEW_HEIGHT)
}

fn preview_app() -> App {
    App::with_missing_optional_deps(
        vec![
            PackageInfo {
                name: "pacman".to_owned(),
                version: "7.0.0-2".to_owned(),
                description: Some(
                    "A library-based package manager with dependency support.".to_owned(),
                ),
                optional_deps: vec![
                    OptionalDep {
                        name: "perl-locale-gettext".to_owned(),
                        optional_for: "translation support".to_owned(),
                        installed_package: Some(InstalledPackage {
                            name: "perl-locale-gettext".to_owned(),
                            version: "1.07-15".to_owned(),
                        }),
                    },
                    OptionalDep {
                        name: "rsync".to_owned(),
                        optional_for: "download mirrors with rsync".to_owned(),
                        installed_package: None,
                    },
                ],
            },
            PackageInfo {
                name: "ripgrep".to_owned(),
                version: "14.1.1-1".to_owned(),
                description: Some(
                    "A search tool that recursively searches directories for a regex pattern."
                        .to_owned(),
                ),
                optional_deps: Vec::new(),
            },
            PackageInfo {
                name: "alacritty".to_owned(),
                version: "0.15.1-1".to_owned(),
                description: Some("A cross-platform, OpenGL terminal emulator.".to_owned()),
                optional_deps: vec![OptionalDep {
                    name: "ncurses".to_owned(),
                    optional_for: "terminfo database entries".to_owned(),
                    installed_package: Some(InstalledPackage {
                        name: "ncurses".to_owned(),
                        version: "6.5-4".to_owned(),
                    }),
                }],
            },
        ],
        vec![
            MissingOptionalDep {
                name: "imagemagick".to_owned(),
                version: Some("7.1.2.0-1".to_owned()),
                description: Some("An image viewing and manipulation program.".to_owned()),
                wanted_by: vec![OptionalDepRequester {
                    package_name: "pacgraph".to_owned(),
                    reason: "png output support".to_owned(),
                }],
            },
            MissingOptionalDep {
                name: "ripgrep-all".to_owned(),
                version: Some("0.10.9-1".to_owned()),
                description: Some(
                    "Search PDFs, archives, ebooks, office documents, and more with ripgrep."
                        .to_owned(),
                ),
                wanted_by: vec![
                    OptionalDepRequester {
                        package_name: "fzf".to_owned(),
                        reason: "faster preview scanning".to_owned(),
                    },
                    OptionalDepRequester {
                        package_name: "document-search".to_owned(),
                        reason: "drop-in replacement backend".to_owned(),
                    },
                ],
            },
            MissingOptionalDep {
                name: "wl-clipboard".to_owned(),
                version: Some("1:2.2.1-3".to_owned()),
                description: Some("Command-line copy and paste utilities for Wayland.".to_owned()),
                wanted_by: vec![OptionalDepRequester {
                    package_name: "neovim".to_owned(),
                    reason: "clipboard support".to_owned(),
                }],
            },
            MissingOptionalDep {
                name: "zoxide".to_owned(),
                version: Some("0.9.8-1".to_owned()),
                description: Some("A smarter cd command inspired by z and autojump.".to_owned()),
                wanted_by: vec![OptionalDepRequester {
                    package_name: "shell-tools".to_owned(),
                    reason: "quick directory jumping".to_owned(),
                }],
            },
        ],
    )
}

fn render_app(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| frame.render_widget(app, frame.area()))
        .unwrap();

    buffer_string(terminal.backend().buffer(), width)
}

fn buffer_string(buffer: &Buffer, width: u16) -> String {
    let mut rendered = String::new();

    for row in buffer.content().chunks(usize::from(width)) {
        let mut line = String::new();
        for cell in row {
            line.push_str(cell.symbol());
        }
        rendered.push_str(line.trim_end());
        rendered.push('\n');
    }

    rendered
}

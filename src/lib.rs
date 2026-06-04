use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use alpm::{Alpm, Dep, DepMod};
use eyre::Result;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub optional_deps: Vec<OptionalDep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDep {
    pub name: String,
    pub optional_for: String,
    pub installed_package: Option<InstalledPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    packages: Vec<PackageInfo>,
    list_state: ListState,
    search_query: String,
    search_active: bool,
    running: bool,
}

impl App {
    pub fn load() -> Result<Self> {
        Ok(Self::new(Self::load_installed_packages()?))
    }

    pub fn new(packages: Vec<PackageInfo>) -> Self {
        let mut list_state = ListState::default();

        if !packages.is_empty() {
            list_state.select_first();
        }

        Self {
            packages,
            list_state,
            search_query: String::new(),
            search_active: false,
            running: true,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let rx = Runtime::start();

        while self.running {
            terminal.draw(|frame| frame.render_widget(&mut *self, frame.area()))?;
            self.update(rx.recv()?);
        }

        Ok(())
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Key(key) => self.handle_key(key),
            Msg::SelectNext => self.select_next(),
            Msg::SelectPrevious => self.select_previous(),
            Msg::Quit => self.running = false,
            Msg::NoOp => {}
        }
    }

    pub fn packages(&self) -> &[PackageInfo] {
        &self.packages
    }

    pub fn selected_package(&self) -> Option<&PackageInfo> {
        let selected = self.selected_index()?;
        let package_index = self.filtered_package_indices().get(selected).copied()?;

        self.packages.get(package_index)
    }

    fn load_installed_packages() -> Result<Vec<PackageInfo>> {
        let alpm = Alpm::new("/", "/var/lib/pacman")?;
        let local_packages = alpm.localdb().pkgs();
        let mut packages = local_packages
            .iter()
            .map(|pkg| PackageInfo {
                name: pkg.name().to_owned(),
                version: pkg.version().to_string(),
                description: pkg.desc().map(str::to_owned),
                optional_deps: pkg
                    .optdepends()
                    .into_iter()
                    .map(|dep| {
                        let requirement = Self::dep_requirement(dep);

                        OptionalDep {
                            name: requirement.clone(),
                            optional_for: dep.desc().map(str::to_owned).unwrap_or_default(),
                            installed_package: local_packages.find_satisfier(requirement).map(
                                |pkg| InstalledPackage {
                                    name: pkg.name().to_owned(),
                                    version: pkg.version().to_string(),
                                },
                            ),
                        }
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();

        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(packages)
    }

    fn dep_requirement(dep: &Dep) -> String {
        match dep.depmod() {
            DepMod::Any => dep.name().to_owned(),
            DepMod::Eq => format!(
                "{}={}",
                dep.name(),
                dep.version().expect("missing dep version")
            ),
            DepMod::Ge => format!(
                "{}>={}",
                dep.name(),
                dep.version().expect("missing dep version")
            ),
            DepMod::Le => format!(
                "{}<={}",
                dep.name(),
                dep.version().expect("missing dep version")
            ),
            DepMod::Gt => format!(
                "{}>{}",
                dep.name(),
                dep.version().expect("missing dep version")
            ),
            DepMod::Lt => format!(
                "{}<{}",
                dep.name(),
                dep.version().expect("missing dep version")
            ),
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn select_next(&mut self) {
        if let Some(last) = self.filtered_package_indices().len().checked_sub(1) {
            let selected = self.selected_index().unwrap_or(0);
            self.list_state.select(Some((selected + 1).min(last)));
        }
    }

    fn select_previous(&mut self) {
        if self.filtered_package_indices().is_empty() {
            return;
        }

        let selected = self.selected_index().unwrap_or(0);
        self.list_state.select(Some(selected.saturating_sub(1)));
    }

    fn package_items(&self) -> Vec<ListItem<'static>> {
        self.filtered_packages()
            .into_iter()
            .map(PackageInfo::list_item)
            .collect::<Vec<_>>()
    }

    fn details(&self) -> Vec<Line<'static>> {
        match self.selected_package() {
            Some(package) => package.details(),
            None => vec![Line::from("No installed packages found.")],
        }
    }

    fn filtered_packages(&self) -> Vec<&PackageInfo> {
        self.filtered_package_indices()
            .into_iter()
            .filter_map(|index| self.packages.get(index))
            .collect::<Vec<_>>()
    }

    fn filtered_package_indices(&self) -> Vec<usize> {
        let query = self.search_query.trim();

        if query.is_empty() {
            return (0..self.packages.len()).collect::<Vec<_>>();
        }

        let query = query.to_lowercase();

        self.packages
            .iter()
            .enumerate()
            .filter_map(|(index, package)| package.matches(&query).then_some(index))
            .collect::<Vec<_>>()
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.search_active {
            self.handle_search_key(key);
        } else {
            self.handle_normal_key(key);
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('/') => self.search_active = true,
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.search_active = false,
            KeyCode::Esc => {
                self.search_query.clear();
                self.search_active = false;
                self.sync_selection_to_filter();
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.sync_selection_to_filter();
            }
            KeyCode::Down => self.select_next(),
            KeyCode::Up => self.select_previous(),
            KeyCode::Char(char) => {
                self.search_query.push(char);
                self.sync_selection_to_filter();
            }
            _ => {}
        }
    }

    fn sync_selection_to_filter(&mut self) {
        let matches = self.filtered_package_indices();

        match (matches.is_empty(), self.selected_index()) {
            (true, _) => self.list_state.select(None),
            (false, Some(selected)) if selected < matches.len() => {}
            (false, _) => self.list_state.select_first(),
        }
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [search_area, content_area, help_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .areas(area);
        let [list_area, details_area] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .areas(content_area);

        let items = self.package_items();
        let details = self.details();
        let match_count = self.filtered_package_indices().len();
        let title = format!(
            " Installed packages ({match_count}/{}) ",
            self.packages.len()
        );
        let search_style = if self.search_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .highlight_symbol("> ")
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        Paragraph::new(self.search_line())
            .style(search_style)
            .render(search_area, buf);

        StatefulWidget::render(list, list_area, buf, &mut self.list_state);

        Paragraph::new(details)
            .block(
                Block::default()
                    .title(" Details ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .render(details_area, buf);

        Paragraph::new("/ search   Up/Down or k/j navigate   q or Esc quit")
            .style(Style::default().fg(Color::DarkGray))
            .render(help_area, buf);
    }
}

impl App {
    fn search_line(&self) -> Line<'static> {
        let cursor = if self.search_active { "_" } else { "" };

        Line::from(vec![
            Span::styled("/", Style::default().fg(Color::DarkGray)),
            Span::raw(self.search_query.clone()),
            Span::styled(cursor, Style::default().fg(Color::Cyan)),
        ])
    }
}

impl PackageInfo {
    fn list_item(&self) -> ListItem<'static> {
        ListItem::new(Line::from(vec![
            Span::styled(
                self.name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(self.version.clone(), Style::default().fg(Color::DarkGray)),
        ]))
    }

    fn details(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    self.name.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(self.version.clone(), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(
                self.description
                    .clone()
                    .unwrap_or_else(|| "No package description available.".to_owned()),
            ),
        ];

        if self.optional_deps.is_empty() {
            return lines;
        }

        lines.push(Line::from(""));
        lines.push(Line::from("Optional deps:"));

        for dep in &self.optional_deps {
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(dep.name.clone(), Style::default().fg(Color::Yellow)),
                Span::raw(": "),
                Span::from(dep.reason()),
                Span::raw("  "),
            ];
            spans.extend(dep.installed_status_spans());
            lines.push(Line::from(spans));
        }

        lines
    }

    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.version.to_lowercase().contains(query)
            || self
                .description
                .as_deref()
                .is_some_and(|description| description.to_lowercase().contains(query))
            || self.optional_deps.iter().any(|dep| dep.matches(query))
    }
}

impl OptionalDep {
    fn reason(&self) -> String {
        if self.optional_for.is_empty() {
            "No reason provided.".to_owned()
        } else {
            self.optional_for.clone()
        }
    }

    fn installed_status_spans(&self) -> Vec<Span<'static>> {
        match &self.installed_package {
            Some(package) => vec![
                Span::styled("satisfied by: ", Style::default().fg(Color::DarkGray)),
                Span::styled(package.name.clone(), Style::default().fg(Color::Green)),
                Span::styled(
                    format!(" {}", package.version),
                    Style::default().fg(Color::DarkGray),
                ),
            ],
            None => vec![Span::styled(
                "not installed",
                Style::default().fg(Color::DarkGray),
            )],
        }
    }

    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.optional_for.to_lowercase().contains(query)
            || self
                .installed_package
                .as_ref()
                .is_some_and(|package| package.matches(query))
    }
}

impl InstalledPackage {
    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query) || self.version.to_lowercase().contains(query)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Key(KeyEvent),
    SelectNext,
    SelectPrevious,
    Quit,
    NoOp,
}

pub fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    App::load()?.run(terminal)
}

struct Runtime;

impl Runtime {
    fn start() -> Receiver<Msg> {
        let (tx, rx) = mpsc::channel();
        Self::spawn_input_thread(tx);
        rx
    }

    fn spawn_input_thread(tx: Sender<Msg>) {
        thread::spawn(move || {
            while let Ok(msg) = Self::read_input_msg() {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
    }

    fn read_input_msg() -> Result<Msg> {
        loop {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    return Ok(Msg::Key(key));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_details_include_optional_dependency_reasons() {
        let package = PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: Some("Example package".to_owned()),
            optional_deps: vec![OptionalDep {
                name: "sqlite".to_owned(),
                optional_for: "database support".to_owned(),
                installed_package: Some(InstalledPackage {
                    name: "sqlite".to_owned(),
                    version: "3.51.1-1".to_owned(),
                }),
            }],
        };

        let rendered = package
            .details()
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(
            rendered
                .contains(&"  sqlite: database support  satisfied by: sqlite 3.51.1-1".to_owned())
        );
    }

    #[test]
    fn package_details_hide_optional_dependency_section_when_empty() {
        let package = PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: Vec::new(),
        };

        let rendered = package
            .details()
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(!rendered.contains(&"Optional deps:".to_owned()));
        assert!(!rendered.contains(&"  None".to_owned()));
    }

    #[test]
    fn package_details_show_uninstalled_optional_dependencies() {
        let package = PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: vec![OptionalDep {
                name: "mysql".to_owned(),
                optional_for: "database support".to_owned(),
                installed_package: None,
            }],
        };

        let rendered = package
            .details()
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.contains(&"  mysql: database support  not installed".to_owned()));
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use alpm::{Alpm, Dep, DepMod, SigLevel};
use eyre::Result;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
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
pub struct MissingOptionalDep {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub wanted_by: Vec<OptionalDepRequester>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDepRequester {
    pub package_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    packages: Vec<PackageInfo>,
    missing_optional_deps: Vec<MissingOptionalDep>,
    checked_missing_optional_deps: BTreeSet<String>,
    selected_missing_optional_dep_range: Option<SelectionRange>,
    mouse_drag_check_action: Option<CheckAction>,
    mouse_drag_position: Option<MousePosition>,
    package_list_state: ListState,
    missing_optional_dep_list_state: ListState,
    active_view: AppView,
    last_list_area: Option<Rect>,
    search_query: String,
    search_active: bool,
    running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Packages,
    MissingOptionalDeps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionRange {
    anchor: usize,
    head: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckAction {
    Check,
    Uncheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MousePosition {
    column: u16,
    row: u16,
}

const IGNORED_MISSING_OPTIONAL_DEP_PREFIXES: &[&str] = &["tesseract-data-", "tesarract-data-"];
const IGNORED_MISSING_OPTIONAL_DEPS: &[&str] = &["tesseract-data", "tesarract-data"];

impl App {
    pub fn load() -> Result<Self> {
        let (packages, missing_optional_deps) = Self::load_package_data()?;

        Ok(Self::with_missing_optional_deps(
            packages,
            missing_optional_deps,
        ))
    }

    pub fn new(packages: Vec<PackageInfo>) -> Self {
        let missing_optional_deps = Self::missing_optional_deps_from_packages(&packages);

        Self::with_missing_optional_deps(packages, missing_optional_deps)
    }

    pub fn with_missing_optional_deps(
        packages: Vec<PackageInfo>,
        missing_optional_deps: Vec<MissingOptionalDep>,
    ) -> Self {
        let mut package_list_state = ListState::default();
        let mut missing_optional_dep_list_state = ListState::default();

        if !packages.is_empty() {
            package_list_state.select_first();
        }

        if !missing_optional_deps.is_empty() {
            missing_optional_dep_list_state.select_first();
        }

        Self {
            packages,
            missing_optional_deps,
            checked_missing_optional_deps: BTreeSet::new(),
            selected_missing_optional_dep_range: None,
            mouse_drag_check_action: None,
            mouse_drag_position: None,
            package_list_state,
            missing_optional_dep_list_state,
            active_view: AppView::Packages,
            last_list_area: None,
            search_query: String::new(),
            search_active: false,
            running: true,
        }
    }

    pub fn run<B>(&mut self, terminal: &mut Terminal<B>) -> Result<Vec<String>>
    where
        B: Backend<Error = io::Error>,
    {
        let _mouse_capture = MouseCaptureGuard::enable()?;
        let rx = Runtime::start();

        while self.running {
            terminal.draw(|frame| frame.render_widget(&mut *self, frame.area()))?;
            self.update(rx.recv()?);
        }

        Ok(self.checked_missing_optional_dep_names())
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Key(key) => self.handle_key(key),
            Msg::Mouse(mouse) => self.handle_mouse(mouse),
            Msg::SelectNext => self.select_next(),
            Msg::SelectPrevious => self.select_previous(),
            Msg::Quit => self.running = false,
            Msg::NoOp => {}
        }
    }

    pub fn packages(&self) -> &[PackageInfo] {
        &self.packages
    }

    pub fn missing_optional_deps(&self) -> &[MissingOptionalDep] {
        &self.missing_optional_deps
    }

    pub fn checked_missing_optional_dep_names(&self) -> Vec<String> {
        self.checked_missing_optional_deps
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn selected_package(&self) -> Option<&PackageInfo> {
        let selected = self.package_list_state.selected()?;
        let package_index = self.filtered_package_indices().get(selected).copied()?;

        self.packages.get(package_index)
    }

    pub fn selected_missing_optional_dep(&self) -> Option<&MissingOptionalDep> {
        let selected = self.missing_optional_dep_list_state.selected()?;
        let index = self
            .filtered_missing_optional_dep_indices()
            .get(selected)
            .copied()?;

        self.missing_optional_deps.get(index)
    }

    fn load_package_data() -> Result<(Vec<PackageInfo>, Vec<MissingOptionalDep>)> {
        let alpm = Alpm::new("/", "/var/lib/pacman")?;
        Self::register_syncdbs(&alpm);

        let local_packages = alpm.localdb().pkgs();
        let syncdbs = alpm.syncdbs();
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
                            installed_package: Self::resolve_installed_optional_dep(
                                &local_packages,
                                dep,
                                requirement.as_str(),
                            ),
                        }
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();

        packages.sort_by(|a, b| a.name.cmp(&b.name));
        let missing_optional_deps =
            Self::missing_optional_deps_from_packages_and_syncdbs(&packages, &syncdbs);

        Ok((packages, missing_optional_deps))
    }

    fn register_syncdbs(alpm: &Alpm) {
        let Ok(entries) = fs::read_dir("/var/lib/pacman/sync") else {
            return;
        };

        for entry in entries.filter_map(|entry| entry.ok()) {
            let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(db_name) = file_name.strip_suffix(".db") else {
                continue;
            };

            let _ = alpm.register_syncdb(db_name, SigLevel::NONE);
        }
    }

    fn missing_optional_deps_from_packages(packages: &[PackageInfo]) -> Vec<MissingOptionalDep> {
        Self::missing_optional_deps_from_packages_with_resolver(packages, |_| None)
    }

    fn missing_optional_deps_from_packages_and_syncdbs(
        packages: &[PackageInfo],
        syncdbs: &alpm::AlpmList<'_, &alpm::Db>,
    ) -> Vec<MissingOptionalDep> {
        Self::missing_optional_deps_from_packages_with_resolver(packages, |dep| {
            syncdbs
                .find_satisfier(dep.name.as_str())
                .map(|pkg| MissingOptionalDep {
                    name: pkg.name().to_owned(),
                    version: Some(pkg.version().to_string()),
                    description: pkg.desc().map(str::to_owned),
                    wanted_by: Vec::new(),
                })
        })
    }

    fn resolve_installed_optional_dep(
        local_packages: &alpm::AlpmList<'_, &alpm::Package>,
        dep: &Dep,
        requirement: &str,
    ) -> Option<InstalledPackage> {
        local_packages
            .find_satisfier(requirement)
            .or_else(|| Self::find_installed_replacement(local_packages, dep))
            .map(Self::installed_package_from_alpm)
    }

    fn find_installed_replacement<'a>(
        local_packages: &alpm::AlpmList<'_, &'a alpm::Package>,
        dep: &Dep,
    ) -> Option<&'a alpm::Package> {
        local_packages.iter().find(|pkg| {
            pkg.conflicts()
                .iter()
                .any(|conflict| Self::dep_names_match(conflict, dep))
                || pkg
                    .replaces()
                    .iter()
                    .any(|replacement| Self::dep_names_match(replacement, dep))
        })
    }

    fn dep_names_match(candidate: &Dep, requested: &Dep) -> bool {
        candidate.name() == requested.name()
    }

    fn installed_package_from_alpm(pkg: &alpm::Package) -> InstalledPackage {
        InstalledPackage {
            name: pkg.name().to_owned(),
            version: pkg.version().to_string(),
        }
    }

    fn missing_optional_deps_from_packages_with_resolver<F>(
        packages: &[PackageInfo],
        mut resolve: F,
    ) -> Vec<MissingOptionalDep>
    where
        F: FnMut(&OptionalDep) -> Option<MissingOptionalDep>,
    {
        let mut deps = BTreeMap::<String, MissingOptionalDep>::new();

        for package in packages {
            for dep in package
                .optional_deps
                .iter()
                .filter(|dep| dep.installed_package.is_none())
            {
                let mut resolved = resolve(dep).unwrap_or_else(|| MissingOptionalDep {
                    name: dep.name.clone(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                });
                if Self::is_ignored_missing_optional_dep(&resolved.name) {
                    continue;
                }

                let key = resolved.name.clone();

                deps.entry(key)
                    .or_insert_with(|| {
                        resolved.wanted_by.clear();
                        resolved
                    })
                    .wanted_by
                    .push(OptionalDepRequester {
                        package_name: package.name.clone(),
                        reason: dep.reason(),
                    });
            }
        }

        let mut deps = deps
            .into_values()
            .map(|mut dep| {
                dep.wanted_by
                    .sort_by(|a, b| a.package_name.cmp(&b.package_name));
                dep
            })
            .collect::<Vec<_>>();

        deps.sort_by(|a, b| {
            b.wanted_by
                .len()
                .cmp(&a.wanted_by.len())
                .then_with(|| a.name.cmp(&b.name))
        });

        deps
    }

    fn is_ignored_missing_optional_dep(name: &str) -> bool {
        IGNORED_MISSING_OPTIONAL_DEPS.contains(&name)
            || IGNORED_MISSING_OPTIONAL_DEP_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
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

    fn active_selected_index(&self) -> Option<usize> {
        match self.active_view {
            AppView::Packages => self.package_list_state.selected(),
            AppView::MissingOptionalDeps => self.missing_optional_dep_list_state.selected(),
        }
    }

    fn select_next(&mut self) {
        if self.mouse_drag_check_action.is_some() {
            self.scroll_active_list_offset(1);
            return;
        }

        self.clear_missing_optional_dep_range();

        if let Some(last) = self.active_filtered_len().checked_sub(1) {
            let selected = self.active_selected_index().unwrap_or(0);
            self.active_list_state()
                .select(Some((selected + 1).min(last)));
        }
    }

    fn select_previous(&mut self) {
        if self.mouse_drag_check_action.is_some() {
            self.scroll_active_list_offset(-1);
            return;
        }

        self.clear_missing_optional_dep_range();

        if self.active_filtered_len() == 0 {
            return;
        }

        let selected = self.active_selected_index().unwrap_or(0);
        self.active_list_state()
            .select(Some(selected.saturating_sub(1)));
    }

    fn active_list_state(&mut self) -> &mut ListState {
        match self.active_view {
            AppView::Packages => &mut self.package_list_state,
            AppView::MissingOptionalDeps => &mut self.missing_optional_dep_list_state,
        }
    }

    fn scroll_active_list_offset(&mut self, direction: isize) {
        let len = self.active_filtered_len();
        if len == 0 {
            return;
        }

        let offset = self.active_list_state().offset_mut();
        if direction.is_negative() {
            *offset = offset.saturating_sub(1);
        } else {
            *offset = offset.saturating_add(1).min(len - 1);
        }

        self.apply_held_mouse_drag_at_current_position();
    }

    fn active_filtered_len(&self) -> usize {
        match self.active_view {
            AppView::Packages => self.filtered_package_indices().len(),
            AppView::MissingOptionalDeps => self.filtered_missing_optional_dep_indices().len(),
        }
    }

    fn package_items(&self) -> Vec<ListItem<'static>> {
        self.filtered_packages()
            .into_iter()
            .map(PackageInfo::list_item)
            .collect::<Vec<_>>()
    }

    fn missing_optional_dep_items(&self) -> Vec<ListItem<'static>> {
        self.filtered_missing_optional_deps()
            .into_iter()
            .enumerate()
            .map(|(position, dep)| {
                dep.list_item(
                    self.checked_missing_optional_deps.contains(&dep.name),
                    self.is_missing_optional_dep_position_in_range(position),
                )
            })
            .collect::<Vec<_>>()
    }

    fn details(&self) -> Vec<Line<'static>> {
        match self.active_view {
            AppView::Packages => match self.selected_package() {
                Some(package) => package.details(),
                None => vec![Line::from("No installed packages found.")],
            },
            AppView::MissingOptionalDeps => match self.selected_missing_optional_dep() {
                Some(dep) => dep.details(),
                None => vec![Line::from("No uninstalled optional dependencies found.")],
            },
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

    fn filtered_missing_optional_deps(&self) -> Vec<&MissingOptionalDep> {
        self.filtered_missing_optional_dep_indices()
            .into_iter()
            .filter_map(|index| self.missing_optional_deps.get(index))
            .collect::<Vec<_>>()
    }

    fn filtered_missing_optional_dep_indices(&self) -> Vec<usize> {
        let query = self.search_query.trim();

        if query.is_empty() {
            return (0..self.missing_optional_deps.len()).collect::<Vec<_>>();
        }

        let query = query.to_lowercase();

        self.missing_optional_deps
            .iter()
            .enumerate()
            .filter_map(|(index, dep)| dep.matches(&query).then_some(index))
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
            KeyCode::Tab => self.switch_view(),
            KeyCode::Char(' ') => self.toggle_selected_missing_optional_dep_checked(),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.extend_missing_optional_dep_range(1);
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.extend_missing_optional_dep_range(-1);
            }
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
            KeyCode::Tab => {
                self.switch_view();
                self.sync_selection_to_filter();
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.extend_missing_optional_dep_range(1);
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.extend_missing_optional_dep_range(-1);
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

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.handle_left_mouse_down(mouse),
            MouseEventKind::Drag(MouseButton::Left) => self.handle_left_mouse_drag(mouse),
            MouseEventKind::Up(MouseButton::Left) => self.clear_mouse_drag_check_action(),
            MouseEventKind::ScrollDown => self.select_next(),
            MouseEventKind::ScrollUp => self.select_previous(),
            _ => {}
        }
    }

    fn handle_left_mouse_down(&mut self, mouse: MouseEvent) {
        self.clear_mouse_drag_check_action();

        let Some(filtered_position) = self.filtered_position_at_mouse(mouse) else {
            return;
        };

        if mouse.modifiers.contains(KeyModifiers::SHIFT)
            && self.active_view == AppView::MissingOptionalDeps
        {
            self.extend_missing_optional_dep_range_to(filtered_position);
            return;
        }

        self.clear_missing_optional_dep_range();
        self.active_list_state().select(Some(filtered_position));

        if self.active_view == AppView::MissingOptionalDeps && self.mouse_hit_checkbox(mouse) {
            let action = self.check_action_for_missing_optional_dep_position(filtered_position);
            self.apply_check_action_to_missing_optional_dep_position(filtered_position, action);
            self.mouse_drag_check_action = Some(action);
            self.mouse_drag_position = Some(mouse.into());
        }
    }

    fn handle_left_mouse_drag(&mut self, mouse: MouseEvent) {
        self.mouse_drag_position = Some(mouse.into());
        let Some(action) = self.mouse_drag_check_action else {
            return;
        };
        let Some(filtered_position) = self.filtered_position_at_mouse(mouse) else {
            return;
        };

        if self.active_view != AppView::MissingOptionalDeps {
            return;
        }

        self.clear_missing_optional_dep_range();
        self.missing_optional_dep_list_state
            .select(Some(filtered_position));
        self.apply_check_action_to_missing_optional_dep_position(filtered_position, action);
    }

    fn apply_held_mouse_drag_at_current_position(&mut self) {
        let Some(action) = self.mouse_drag_check_action else {
            return;
        };
        let Some(position) = self.mouse_drag_position else {
            return;
        };
        let Some(filtered_position) = self.filtered_position_at_mouse(position) else {
            return;
        };

        if self.active_view != AppView::MissingOptionalDeps {
            return;
        }

        self.clear_missing_optional_dep_range();
        self.missing_optional_dep_list_state
            .select(Some(filtered_position));
        self.apply_check_action_to_missing_optional_dep_position(filtered_position, action);
    }

    fn clear_mouse_drag_check_action(&mut self) {
        self.mouse_drag_check_action = None;
        self.mouse_drag_position = None;
    }

    fn switch_view(&mut self) {
        self.clear_mouse_drag_check_action();
        self.active_view = match self.active_view {
            AppView::Packages => AppView::MissingOptionalDeps,
            AppView::MissingOptionalDeps => AppView::Packages,
        };
        self.clear_missing_optional_dep_range();
        self.sync_selection_to_filter();
    }

    fn toggle_selected_missing_optional_dep_checked(&mut self) {
        if self.active_view != AppView::MissingOptionalDeps {
            return;
        }

        if self.selected_missing_optional_dep_range.is_some() {
            self.toggle_selected_missing_optional_dep_range_checked();
        } else {
            self.toggle_single_selected_missing_optional_dep_checked();
        }
    }

    fn toggle_single_selected_missing_optional_dep_checked(&mut self) {
        let Some(name) = self
            .selected_missing_optional_dep()
            .map(|dep| dep.name.clone())
        else {
            return;
        };

        if !self.checked_missing_optional_deps.insert(name.clone()) {
            self.checked_missing_optional_deps.remove(&name);
        }
    }

    fn check_action_for_missing_optional_dep_position(
        &self,
        filtered_position: usize,
    ) -> CheckAction {
        if self
            .missing_optional_dep_name_at_filtered_position(filtered_position)
            .is_some_and(|name| self.checked_missing_optional_deps.contains(name))
        {
            CheckAction::Uncheck
        } else {
            CheckAction::Check
        }
    }

    fn apply_check_action_to_missing_optional_dep_position(
        &mut self,
        filtered_position: usize,
        action: CheckAction,
    ) {
        let Some(name) = self
            .missing_optional_dep_name_at_filtered_position(filtered_position)
            .map(str::to_owned)
        else {
            return;
        };

        match action {
            CheckAction::Check => {
                self.checked_missing_optional_deps.insert(name);
            }
            CheckAction::Uncheck => {
                self.checked_missing_optional_deps.remove(&name);
            }
        }
    }

    fn missing_optional_dep_name_at_filtered_position(
        &self,
        filtered_position: usize,
    ) -> Option<&str> {
        let index = self
            .filtered_missing_optional_dep_indices()
            .get(filtered_position)
            .copied()?;

        self.missing_optional_deps
            .get(index)
            .map(|dep| dep.name.as_str())
    }

    fn toggle_selected_missing_optional_dep_range_checked(&mut self) {
        let names = self.selected_missing_optional_dep_range_names();
        if names.is_empty() {
            return;
        }

        let all_checked = names
            .iter()
            .all(|name| self.checked_missing_optional_deps.contains(name));

        for name in names {
            if all_checked {
                self.checked_missing_optional_deps.remove(&name);
            } else {
                self.checked_missing_optional_deps.insert(name);
            }
        }
    }

    fn extend_missing_optional_dep_range(&mut self, direction: isize) {
        if self.mouse_drag_check_action.is_some() {
            self.scroll_active_list_offset(direction);
            return;
        }

        if self.active_view != AppView::MissingOptionalDeps {
            self.select_by_direction(direction);
            return;
        }

        let len = self.filtered_missing_optional_dep_indices().len();
        if len == 0 {
            self.missing_optional_dep_list_state.select(None);
            self.selected_missing_optional_dep_range = None;
            return;
        }

        let current = self
            .missing_optional_dep_list_state
            .selected()
            .unwrap_or(0)
            .min(len - 1);
        let next = if direction.is_negative() {
            current.saturating_sub(1)
        } else {
            (current + 1).min(len - 1)
        };

        self.extend_missing_optional_dep_range_to(next);
    }

    fn select_by_direction(&mut self, direction: isize) {
        if direction.is_negative() {
            self.select_previous();
        } else {
            self.select_next();
        }
    }

    fn extend_missing_optional_dep_range_to(&mut self, head: usize) {
        let len = self.filtered_missing_optional_dep_indices().len();
        if len == 0 {
            self.missing_optional_dep_list_state.select(None);
            self.selected_missing_optional_dep_range = None;
            return;
        }

        let head = head.min(len - 1);
        let anchor = self
            .selected_missing_optional_dep_range
            .map(|range| range.anchor)
            .or_else(|| self.missing_optional_dep_list_state.selected())
            .unwrap_or(head)
            .min(len - 1);

        self.selected_missing_optional_dep_range = Some(SelectionRange { anchor, head });
        self.missing_optional_dep_list_state.select(Some(head));
    }

    fn selected_missing_optional_dep_range_names(&self) -> Vec<String> {
        let Some(range) = self.selected_missing_optional_dep_range else {
            return Vec::new();
        };

        let filtered_indices = self.filtered_missing_optional_dep_indices();

        range
            .positions()
            .filter_map(|position| filtered_indices.get(position).copied())
            .filter_map(|index| self.missing_optional_deps.get(index))
            .map(|dep| dep.name.clone())
            .collect::<Vec<_>>()
    }

    fn is_missing_optional_dep_position_in_range(&self, position: usize) -> bool {
        self.selected_missing_optional_dep_range
            .is_some_and(|range| range.contains(position))
    }

    fn clear_missing_optional_dep_range(&mut self) {
        self.selected_missing_optional_dep_range = None;
    }

    fn missing_optional_dep_range_bounds(start: usize, end: usize) -> (usize, usize) {
        if start <= end {
            (start, end)
        } else {
            (end, start)
        }
    }

    fn filtered_position_at_mouse(&self, mouse: impl Into<MousePosition>) -> Option<usize> {
        let mouse = mouse.into();
        let list_area = self.last_list_area?;
        let inner_x = list_area.x.saturating_add(1);
        let inner_y = list_area.y.saturating_add(1);
        let inner_right = list_area
            .x
            .saturating_add(list_area.width.saturating_sub(1));
        let inner_bottom = list_area
            .y
            .saturating_add(list_area.height.saturating_sub(1));

        if mouse.column < inner_x
            || mouse.column >= inner_right
            || mouse.row < inner_y
            || mouse.row >= inner_bottom
        {
            return None;
        }

        let visible_position = usize::from(mouse.row.saturating_sub(inner_y));
        let filtered_position = self.active_list_offset().saturating_add(visible_position);

        (filtered_position < self.active_filtered_len()).then_some(filtered_position)
    }

    fn active_list_offset(&self) -> usize {
        match self.active_view {
            AppView::Packages => self.package_list_state.offset(),
            AppView::MissingOptionalDeps => self.missing_optional_dep_list_state.offset(),
        }
    }

    fn mouse_hit_checkbox(&self, mouse: MouseEvent) -> bool {
        let Some(list_area) = self.last_list_area else {
            return false;
        };
        let inner_x = list_area.x.saturating_add(1);

        mouse.column >= inner_x && mouse.column <= inner_x.saturating_add(6)
    }

    fn sync_selection_to_filter(&mut self) {
        self.clear_missing_optional_dep_range();
        self.clear_mouse_drag_check_action();

        let matches = self.active_filtered_len();
        let selected = self.active_selected_index();
        let list_state = self.active_list_state();

        match (matches == 0, selected) {
            (true, _) => list_state.select(None),
            (false, Some(selected)) if selected < matches => {}
            (false, _) => list_state.select_first(),
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
        self.last_list_area = Some(list_area);

        let items = self.active_items();
        let details = self.details();
        let title = self.active_title();
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

        match self.active_view {
            AppView::Packages => {
                StatefulWidget::render(list, list_area, buf, &mut self.package_list_state);
            }
            AppView::MissingOptionalDeps => {
                StatefulWidget::render(
                    list,
                    list_area,
                    buf,
                    &mut self.missing_optional_dep_list_state,
                );
            }
        }

        Paragraph::new(details)
            .block(
                Block::default()
                    .title(" Details ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .render(details_area, buf);

        Paragraph::new(
            "Tab switch view   Space toggle   Shift+Up/Down or Shift+click range   click select/check   / search   q quit",
        )
        .style(Style::default().fg(Color::DarkGray))
        .render(help_area, buf);
    }
}

impl App {
    fn active_items(&self) -> Vec<ListItem<'static>> {
        match self.active_view {
            AppView::Packages => self.package_items(),
            AppView::MissingOptionalDeps => self.missing_optional_dep_items(),
        }
    }

    fn active_title(&self) -> String {
        match self.active_view {
            AppView::Packages => {
                let match_count = self.filtered_package_indices().len();
                format!(
                    " Installed packages ({match_count}/{}) ",
                    self.packages.len()
                )
            }
            AppView::MissingOptionalDeps => {
                let match_count = self.filtered_missing_optional_dep_indices().len();
                format!(
                    " Missing optional deps ({match_count}/{}) ",
                    self.missing_optional_deps.len()
                )
            }
        }
    }

    fn search_line(&self) -> Line<'static> {
        let cursor = if self.search_active { "_" } else { "" };

        Line::from(vec![
            Span::styled(
                self.active_view.label(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" ", Style::default().fg(Color::DarkGray)),
            Span::styled("/", Style::default().fg(Color::DarkGray)),
            Span::raw(self.search_query.clone()),
            Span::styled(cursor, Style::default().fg(Color::Cyan)),
        ])
    }
}

impl AppView {
    fn label(self) -> &'static str {
        match self {
            AppView::Packages => "[packages]",
            AppView::MissingOptionalDeps => "[missing optional deps]",
        }
    }
}

impl SelectionRange {
    fn contains(self, position: usize) -> bool {
        let (start, end) = App::missing_optional_dep_range_bounds(self.anchor, self.head);

        position >= start && position <= end
    }

    fn positions(self) -> impl Iterator<Item = usize> {
        let (start, end) = App::missing_optional_dep_range_bounds(self.anchor, self.head);

        start..=end
    }
}

impl From<MouseEvent> for MousePosition {
    fn from(mouse: MouseEvent) -> Self {
        Self {
            column: mouse.column,
            row: mouse.row,
        }
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

impl MissingOptionalDep {
    fn list_item(&self, checked: bool, range_selected: bool) -> ListItem<'static> {
        let checkbox = if checked { "[x]" } else { "[ ]" };
        let mut spans = vec![
            Span::styled(checkbox, Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::styled(
                self.name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ];

        if let Some(version) = &self.version {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                version.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("wanted by {}", self.wanted_by.len()),
            Style::default().fg(Color::DarkGray),
        ));

        let item = ListItem::new(Line::from(spans));
        if range_selected {
            item.style(Style::default().bg(Color::DarkGray))
        } else {
            item
        }
    }

    fn details(&self) -> Vec<Line<'static>> {
        let mut title = vec![Span::styled(
            self.name.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];

        if let Some(version) = &self.version {
            title.push(Span::raw(" "));
            title.push(Span::styled(
                version.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        let mut lines = vec![
            Line::from(title),
            Line::from(""),
            Line::from(
                self.description
                    .clone()
                    .unwrap_or_else(|| "No package description available.".to_owned()),
            ),
            Line::from(""),
            Line::from("Wanted by:"),
        ];

        for requester in &self.wanted_by {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    requester.package_name.clone(),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(": "),
                Span::from(requester.reason.clone()),
            ]));
        }

        lines
    }

    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self
                .version
                .as_ref()
                .is_some_and(|version| version.to_lowercase().contains(query))
            || self
                .description
                .as_ref()
                .is_some_and(|description| description.to_lowercase().contains(query))
            || self.wanted_by.iter().any(|requester| {
                requester.package_name.to_lowercase().contains(query)
                    || requester.reason.to_lowercase().contains(query)
            })
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
    Mouse(MouseEvent),
    SelectNext,
    SelectPrevious,
    Quit,
    NoOp,
}

pub fn run_app<B>(terminal: &mut Terminal<B>) -> Result<Vec<String>>
where
    B: Backend<Error = io::Error>,
{
    App::load()?.run(terminal)
}

struct Runtime;

struct MouseCaptureGuard;

impl MouseCaptureGuard {
    fn enable() -> Result<Self> {
        execute!(io::stderr(), EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stderr(), DisableMouseCapture);
    }
}

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
                Event::Mouse(mouse) => return Ok(Msg::Mouse(mouse)),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpm::Depend;

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

    #[test]
    fn missing_optional_deps_skip_satisfied_dependencies() {
        let packages = vec![PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: vec![
                OptionalDep {
                    name: "sqlite".to_owned(),
                    optional_for: "database support".to_owned(),
                    installed_package: Some(InstalledPackage {
                        name: "sqlite".to_owned(),
                        version: "3.51.1-1".to_owned(),
                    }),
                },
                OptionalDep {
                    name: "mysql".to_owned(),
                    optional_for: "database support".to_owned(),
                    installed_package: None,
                },
            ],
        }];

        let missing = App::missing_optional_deps_from_packages(&packages);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "mysql");
        assert_eq!(missing[0].wanted_by[0].package_name, "example");
    }

    #[test]
    fn missing_optional_dep_details_include_requesting_packages_and_reasons() {
        let dep = MissingOptionalDep {
            name: "mysql".to_owned(),
            version: Some("8.4.3-1".to_owned()),
            description: Some("SQL database server".to_owned()),
            wanted_by: vec![
                OptionalDepRequester {
                    package_name: "alpha".to_owned(),
                    reason: "database support".to_owned(),
                },
                OptionalDepRequester {
                    package_name: "beta".to_owned(),
                    reason: "mysql backend".to_owned(),
                },
            ],
        };

        let rendered = dep
            .details()
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.contains(&"SQL database server".to_owned()));
        assert!(rendered.contains(&"  alpha: database support".to_owned()));
        assert!(rendered.contains(&"  beta: mysql backend".to_owned()));
    }

    #[test]
    fn replacement_deps_match_requested_optional_dep_names() {
        let replacement = Depend::new("pulseaudio");
        let requested = Depend::new("pulseaudio");
        let unrelated = Depend::new("jack");

        assert!(App::dep_names_match(&replacement, &requested));
        assert!(!App::dep_names_match(&replacement, &unrelated));
    }

    #[test]
    fn missing_optional_deps_sort_by_wanted_count_and_filter_ignored_names() {
        let packages = vec![
            PackageInfo {
                name: "alpha".to_owned(),
                version: "1.0.0".to_owned(),
                description: None,
                optional_deps: vec![
                    OptionalDep {
                        name: "zlib".to_owned(),
                        optional_for: "compression".to_owned(),
                        installed_package: None,
                    },
                    OptionalDep {
                        name: "tesseract-data-eng".to_owned(),
                        optional_for: "OCR language data".to_owned(),
                        installed_package: None,
                    },
                ],
            },
            PackageInfo {
                name: "beta".to_owned(),
                version: "1.0.0".to_owned(),
                description: None,
                optional_deps: vec![
                    OptionalDep {
                        name: "sqlite".to_owned(),
                        optional_for: "database".to_owned(),
                        installed_package: None,
                    },
                    OptionalDep {
                        name: "zlib".to_owned(),
                        optional_for: "compression".to_owned(),
                        installed_package: None,
                    },
                ],
            },
        ];

        let missing = App::missing_optional_deps_from_packages(&packages);

        assert_eq!(
            missing
                .iter()
                .map(|dep| dep.name.as_str())
                .collect::<Vec<_>>(),
            vec!["zlib", "sqlite"]
        );
    }

    #[test]
    fn missing_optional_dep_checkbox_toggles_selected_entry() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![MissingOptionalDep {
                name: "sqlite".to_owned(),
                version: None,
                description: None,
                wanted_by: Vec::new(),
            }],
        );

        app.switch_view();
        app.toggle_selected_missing_optional_dep_checked();
        assert!(app.checked_missing_optional_deps.contains("sqlite"));

        app.toggle_selected_missing_optional_dep_checked();
        assert!(!app.checked_missing_optional_deps.contains("sqlite"));
    }

    #[test]
    fn checked_missing_optional_dep_names_are_returned_sorted() {
        let mut app = App::with_missing_optional_deps(Vec::new(), Vec::new());
        app.checked_missing_optional_deps.insert("zlib".to_owned());
        app.checked_missing_optional_deps
            .insert("sqlite".to_owned());

        assert_eq!(
            app.checked_missing_optional_dep_names(),
            vec!["sqlite".to_owned(), "zlib".to_owned()]
        );
    }

    #[test]
    fn shift_down_selects_range_that_space_can_toggle() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![
                MissingOptionalDep {
                    name: "sqlite".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "zlib".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
            ],
        );

        app.switch_view();
        app.extend_missing_optional_dep_range(1);

        assert!(app.checked_missing_optional_deps.is_empty());
        assert_eq!(
            app.selected_missing_optional_dep_range_names(),
            vec!["sqlite".to_owned(), "zlib".to_owned()]
        );
        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(1));

        app.toggle_selected_missing_optional_dep_checked();
        assert!(app.checked_missing_optional_deps.contains("sqlite"));
        assert!(app.checked_missing_optional_deps.contains("zlib"));

        app.toggle_selected_missing_optional_dep_checked();
        assert!(app.checked_missing_optional_deps.is_empty());
    }

    #[test]
    fn mouse_click_selects_and_checkbox_area_toggles_missing_optional_dep() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![MissingOptionalDep {
                name: "sqlite".to_owned(),
                version: None,
                description: None,
                wanted_by: Vec::new(),
            }],
        );
        app.switch_view();
        app.last_list_area = Some(Rect::new(0, 1, 40, 6));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });

        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(0));
        assert!(app.checked_missing_optional_deps.contains("sqlite"));
    }

    #[test]
    fn mouse_shift_click_selects_range_without_checking_entries() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![
                MissingOptionalDep {
                    name: "sqlite".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "zlib".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "mysql".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
            ],
        );
        app.switch_view();
        app.last_list_area = Some(Rect::new(0, 1, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 4,
            modifiers: KeyModifiers::SHIFT,
        });

        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(2));
        assert!(app.checked_missing_optional_deps.is_empty());
        assert_eq!(
            app.selected_missing_optional_dep_range_names(),
            vec!["sqlite".to_owned(), "zlib".to_owned(), "mysql".to_owned()]
        );

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 3,
            modifiers: KeyModifiers::empty(),
        });

        assert!(app.selected_missing_optional_dep_range.is_none());
        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(1));
        assert!(app.checked_missing_optional_deps.contains("zlib"));
        assert_eq!(app.checked_missing_optional_deps.len(), 1);
    }

    #[test]
    fn mouse_drag_from_checkbox_checks_entries_with_initial_action() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![
                MissingOptionalDep {
                    name: "sqlite".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "zlib".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "mysql".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
            ],
        );
        app.switch_view();
        app.last_list_area = Some(Rect::new(0, 1, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 20,
            row: 3,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 20,
            row: 4,
            modifiers: KeyModifiers::empty(),
        });

        assert!(app.checked_missing_optional_deps.contains("sqlite"));
        assert!(app.checked_missing_optional_deps.contains("zlib"));
        assert!(app.checked_missing_optional_deps.contains("mysql"));
        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(2));
    }

    #[test]
    fn mouse_drag_from_checked_checkbox_unchecks_entries_with_initial_action() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![
                MissingOptionalDep {
                    name: "sqlite".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "zlib".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
            ],
        );
        app.switch_view();
        app.last_list_area = Some(Rect::new(0, 1, 40, 8));
        app.checked_missing_optional_deps
            .insert("sqlite".to_owned());
        app.checked_missing_optional_deps.insert("zlib".to_owned());

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 20,
            row: 3,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 20,
            row: 3,
            modifiers: KeyModifiers::empty(),
        });

        assert!(app.checked_missing_optional_deps.is_empty());
        assert!(app.mouse_drag_check_action.is_none());
    }

    #[test]
    fn keyboard_navigation_scrolls_offset_and_applies_drag_at_held_mouse_position() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![
                MissingOptionalDep {
                    name: "sqlite".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "zlib".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "mysql".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
            ],
        );
        app.switch_view();
        app.last_list_area = Some(Rect::new(0, 1, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));

        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(1));
        assert_eq!(app.missing_optional_dep_list_state.offset(), 1);
        assert!(app.checked_missing_optional_deps.contains("zlib"));

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));

        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(0));
        assert_eq!(app.missing_optional_dep_list_state.offset(), 0);
        assert!(app.mouse_drag_check_action.is_some());
    }

    #[test]
    fn mouse_wheel_scrolls_offset_and_applies_drag_at_held_mouse_position() {
        let mut app = App::with_missing_optional_deps(
            Vec::new(),
            vec![
                MissingOptionalDep {
                    name: "sqlite".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
                MissingOptionalDep {
                    name: "zlib".to_owned(),
                    version: None,
                    description: None,
                    wanted_by: Vec::new(),
                },
            ],
        );
        app.switch_view();
        app.last_list_area = Some(Rect::new(0, 1, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 20,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });

        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(1));
        assert_eq!(app.missing_optional_dep_list_state.offset(), 1);
        assert!(app.checked_missing_optional_deps.contains("zlib"));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 20,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 20,
            row: 2,
            modifiers: KeyModifiers::empty(),
        });

        assert_eq!(app.missing_optional_dep_list_state.selected(), Some(0));
        assert_eq!(app.missing_optional_dep_list_state.offset(), 1);
    }
}

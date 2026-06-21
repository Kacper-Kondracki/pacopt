//! Application state and event handling.
//!
//! `App` owns the package data, current selections, search state, and list
//! navigation state used by the terminal UI.

use std::collections::BTreeSet;
use std::io;

use eyre::Result;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::model::{MissingOptionalDep, PackageInfo};
use crate::pacman;
use crate::runtime::{MouseCaptureGuard, Msg, Runtime};

/// Terminal application state for browsing packages and missing optional dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub(crate) packages: Vec<PackageInfo>,
    pub(crate) missing_optional_deps: Vec<MissingOptionalDep>,
    pub(crate) checked_missing_optional_deps: BTreeSet<String>,
    pub(crate) selected_missing_optional_dep_range: Option<SelectionRange>,
    pub(crate) mouse_drag_check_action: Option<CheckAction>,
    pub(crate) mouse_drag_position: Option<MousePosition>,
    pub(crate) package_list_state: ListState,
    pub(crate) missing_optional_dep_list_state: ListState,
    pub(crate) active_view: AppView,
    pub(crate) last_list_area: Option<Rect>,
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    running: bool,
}

/// Main list currently controlled by keyboard, mouse, and search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    /// Installed package list.
    Packages,
    /// Missing optional dependency list.
    MissingOptionalDeps,
}

/// Inclusive selection range in the filtered missing-dependency list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionRange {
    anchor: usize,
    head: usize,
}

/// Check-state action applied during mouse drag selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckAction {
    /// Mark entries as checked.
    Check,
    /// Mark entries as unchecked.
    Uncheck,
}

/// Mouse coordinates captured from crossterm events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MousePosition {
    column: u16,
    row: u16,
}

impl App {
    /// Loads package data from pacman and creates an application state.
    pub fn load() -> Result<Self> {
        let (packages, missing_optional_deps) = pacman::load_package_data()?;

        Ok(Self::with_missing_optional_deps(
            packages,
            missing_optional_deps,
        ))
    }

    /// Creates an application state from installed package data.
    ///
    /// Missing optional dependencies are derived from `packages` without
    /// consulting sync databases, so version and description metadata are not
    /// populated for missing dependencies.
    pub fn new(packages: Vec<PackageInfo>) -> Self {
        let missing_optional_deps = pacman::missing_optional_deps_from_packages(&packages);

        Self::with_missing_optional_deps(packages, missing_optional_deps)
    }

    /// Creates an application state from precomputed package and missing-dependency data.
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

    /// Runs the draw and input loop until the user quits.
    ///
    /// The returned vector contains the checked missing optional dependency
    /// names in sorted order.
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

    /// Applies one input or control message to the application state.
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

    /// Returns all installed packages currently loaded in the app.
    pub fn packages(&self) -> &[PackageInfo] {
        &self.packages
    }

    /// Returns all missing optional dependencies currently loaded in the app.
    pub fn missing_optional_deps(&self) -> &[MissingOptionalDep] {
        &self.missing_optional_deps
    }

    /// Returns checked missing optional dependency names in sorted order.
    pub fn checked_missing_optional_dep_names(&self) -> Vec<String> {
        self.checked_missing_optional_deps
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    }

    /// Returns the selected package after applying the current search filter.
    pub fn selected_package(&self) -> Option<&PackageInfo> {
        let selected = self.package_list_state.selected()?;
        let package_index = self.filtered_package_indices().get(selected).copied()?;

        self.packages.get(package_index)
    }

    /// Returns the selected missing optional dependency after applying the current search filter.
    pub fn selected_missing_optional_dep(&self) -> Option<&MissingOptionalDep> {
        let selected = self.missing_optional_dep_list_state.selected()?;
        let index = self
            .filtered_missing_optional_dep_indices()
            .get(selected)
            .copied()?;

        self.missing_optional_deps.get(index)
    }

    /// Returns the selected index in the active filtered list.
    pub(crate) fn active_selected_index(&self) -> Option<usize> {
        match self.active_view {
            AppView::Packages => self.package_list_state.selected(),
            AppView::MissingOptionalDeps => self.missing_optional_dep_list_state.selected(),
        }
    }

    /// Moves selection to the next item in the active filtered list.
    pub(crate) fn select_next(&mut self) {
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

    /// Moves selection to the previous item in the active filtered list.
    pub(crate) fn select_previous(&mut self) {
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

    /// Returns mutable list state for the active view.
    pub(crate) fn active_list_state(&mut self) -> &mut ListState {
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

    /// Returns the number of items visible in the active list after filtering.
    pub(crate) fn active_filtered_len(&self) -> usize {
        match self.active_view {
            AppView::Packages => self.filtered_package_indices().len(),
            AppView::MissingOptionalDeps => self.filtered_missing_optional_dep_indices().len(),
        }
    }

    /// Returns package references that match the current search query.
    pub(crate) fn filtered_packages(&self) -> Vec<&PackageInfo> {
        self.filtered_package_indices()
            .into_iter()
            .filter_map(|index| self.packages.get(index))
            .collect::<Vec<_>>()
    }

    /// Returns indices of packages that match the current search query.
    pub(crate) fn filtered_package_indices(&self) -> Vec<usize> {
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

    /// Returns missing optional dependency references that match the current search query.
    pub(crate) fn filtered_missing_optional_deps(&self) -> Vec<&MissingOptionalDep> {
        self.filtered_missing_optional_dep_indices()
            .into_iter()
            .filter_map(|index| self.missing_optional_deps.get(index))
            .collect::<Vec<_>>()
    }

    /// Returns indices of missing optional dependencies that match the current search query.
    pub(crate) fn filtered_missing_optional_dep_indices(&self) -> Vec<usize> {
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

    /// Applies a keyboard event to the current app mode.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
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

    /// Applies a mouse event to list selection, scrolling, or checking.
    pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) {
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

    /// Switches between the installed-package list and missing-dependency list.
    pub(crate) fn switch_view(&mut self) {
        self.clear_mouse_drag_check_action();
        self.active_view = match self.active_view {
            AppView::Packages => AppView::MissingOptionalDeps,
            AppView::MissingOptionalDeps => AppView::Packages,
        };
        self.clear_missing_optional_dep_range();
        self.sync_selection_to_filter();
    }

    /// Toggles the selected missing dependency or selected missing-dependency range.
    pub(crate) fn toggle_selected_missing_optional_dep_checked(&mut self) {
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

    /// Extends the active selection range by one position.
    ///
    /// In the package view this behaves like normal directional selection. In
    /// the missing-dependency view it creates or extends a range that can be
    /// toggled as a group.
    pub(crate) fn extend_missing_optional_dep_range(&mut self, direction: isize) {
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

    /// Returns dependency names included in the current filtered range selection.
    pub(crate) fn selected_missing_optional_dep_range_names(&self) -> Vec<String> {
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

    /// Returns `true` when a filtered missing-dependency position is range-selected.
    pub(crate) fn is_missing_optional_dep_position_in_range(&self, position: usize) -> bool {
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

impl AppView {
    /// Returns the short label shown next to the search prompt.
    pub(crate) fn label(self) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InstalledPackage, OptionalDep};

    fn missing_dep(name: &str) -> MissingOptionalDep {
        MissingOptionalDep {
            name: name.to_owned(),
            version: None,
            description: None,
            wanted_by: Vec::new(),
        }
    }

    #[test]
    fn missing_optional_dep_checkbox_toggles_selected_entry() {
        let mut app = App::with_missing_optional_deps(Vec::new(), vec![missing_dep("sqlite")]);

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
            vec![missing_dep("sqlite"), missing_dep("zlib")],
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
        let mut app = App::with_missing_optional_deps(Vec::new(), vec![missing_dep("sqlite")]);
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
                missing_dep("sqlite"),
                missing_dep("zlib"),
                missing_dep("mysql"),
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
                missing_dep("sqlite"),
                missing_dep("zlib"),
                missing_dep("mysql"),
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
            vec![missing_dep("sqlite"), missing_dep("zlib")],
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
                missing_dep("sqlite"),
                missing_dep("zlib"),
                missing_dep("mysql"),
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
            vec![missing_dep("sqlite"), missing_dep("zlib")],
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

    #[test]
    fn new_builds_missing_optional_deps_from_packages() {
        let app = App::new(vec![PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: vec![OptionalDep {
                name: "sqlite".to_owned(),
                optional_for: "database support".to_owned(),
                installed_package: None,
            }],
        }]);

        assert_eq!(app.missing_optional_deps()[0].name, "sqlite");
    }

    #[test]
    fn package_selection_uses_filtered_indices() {
        let app = App::new(vec![PackageInfo {
            name: "example".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            optional_deps: vec![OptionalDep {
                name: "sqlite".to_owned(),
                optional_for: "database support".to_owned(),
                installed_package: Some(InstalledPackage {
                    name: "sqlite".to_owned(),
                    version: "3.51.1-1".to_owned(),
                }),
            }],
        }]);

        assert_eq!(app.selected_package().unwrap().name, "example");
    }
}

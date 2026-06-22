use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, Paragraph, StatefulWidget, Widget, Wrap,
};

use crate::app::{App, AppView};
use crate::model::{MissingOptionalDep, OptionalDep, PackageInfo};

const BENEFIT_REASON_KEYWORDS: &[&str] = &[
    "fast",
    "faster",
    "quick",
    "quicker",
    "speed",
    "performance",
    "accelerated",
    "acceleration",
    "efficient",
    "efficiency",
    "optimized",
    "optimised",
    "optimize",
    "optimise",
    "optimization",
    "optimisation",
    "better",
    "improved",
    "improvement",
    "enhanced",
    "safer",
    "safe",
    "secure",
    "security",
    "sandbox",
    "sandboxed",
];
const ALTERNATIVE_REASON_KEYWORDS: &[&str] = &[
    "replacement",
    "replace",
    "replaces",
    "replacing",
    "alternative",
    "alternate",
    "instead",
    "fallback",
    "substitute",
    "substitutes",
    "substitution",
    "drop-in",
    "drop in",
    "dropin",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionalReasonSignal {
    Benefit,
    Alternative,
}

const LIST_BLOCK_BORDER_WIDTH: u16 = 2;
const LIST_HIGHLIGHT_SYMBOL_WIDTH: u16 = 2;

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
        let [max_list_area, _] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .areas(content_area);
        let title = self.active_title();
        let signal_column_width = self.active_list_signal_column_width();
        let list_area = Rect {
            width: self.active_list_pane_width(max_list_area.width, &title, signal_column_width),
            ..max_list_area
        };
        let details_area = Rect {
            x: list_area.x.saturating_add(list_area.width),
            width: content_area.width.saturating_sub(list_area.width),
            ..content_area
        };
        self.last_list_area = Some(list_area);
        let list_item_width = list_item_width(list_area, self.active_list_has_selection());

        let items = self.active_items(list_item_width, signal_column_width);
        let details = self.details();
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
            .wrap(Wrap { trim: false })
            .render(details_area, buf);

        Paragraph::new(
            "Tab switch view   Space toggle   Shift+Up/Down or Shift+click range   click select/check   / search   q quit",
        )
        .style(Style::default().fg(Color::DarkGray))
        .render(help_area, buf);
    }
}

impl App {
    fn active_items(
        &self,
        list_item_width: usize,
        signal_column_width: usize,
    ) -> Vec<ListItem<'static>> {
        match self.active_view {
            AppView::Packages => self.package_items(),
            AppView::MissingOptionalDeps => {
                self.missing_optional_dep_items(list_item_width, signal_column_width)
            }
        }
    }

    fn package_items(&self) -> Vec<ListItem<'static>> {
        self.filtered_packages()
            .into_iter()
            .map(package_list_item)
            .collect::<Vec<_>>()
    }

    fn missing_optional_dep_items(
        &self,
        list_item_width: usize,
        signal_column_width: usize,
    ) -> Vec<ListItem<'static>> {
        self.filtered_missing_optional_deps()
            .into_iter()
            .enumerate()
            .map(|(position, dep)| {
                missing_optional_dep_list_item(
                    dep,
                    self.checked_missing_optional_deps.contains(&dep.name),
                    self.is_missing_optional_dep_position_in_range(position),
                    list_item_width,
                    signal_column_width,
                )
            })
            .collect::<Vec<_>>()
    }

    fn details(&self) -> Vec<Line<'static>> {
        match self.active_view {
            AppView::Packages => match self.selected_package() {
                Some(package) => package_details(package),
                None => vec![Line::from("No installed packages found.")],
            },
            AppView::MissingOptionalDeps => match self.selected_missing_optional_dep() {
                Some(dep) => missing_optional_dep_details(dep),
                None => vec![Line::from("No uninstalled optional dependencies found.")],
            },
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

    fn active_list_has_selection(&self) -> bool {
        match self.active_view {
            AppView::Packages => self.package_list_state.selected().is_some(),
            AppView::MissingOptionalDeps => {
                self.missing_optional_dep_list_state.selected().is_some()
            }
        }
    }

    fn active_list_pane_width(
        &self,
        max_width: u16,
        title: &str,
        signal_column_width: usize,
    ) -> u16 {
        let item_width = match self.active_view {
            AppView::Packages => self
                .packages
                .iter()
                .map(package_list_required_width)
                .max()
                .unwrap_or_default(),
            AppView::MissingOptionalDeps => self
                .missing_optional_deps
                .iter()
                .map(|dep| missing_optional_dep_list_required_width(dep, signal_column_width))
                .max()
                .unwrap_or_default(),
        };

        capped_list_pane_width(
            max_width,
            title,
            item_width,
            self.active_list_has_selection(),
        )
    }

    fn active_list_signal_column_width(&self) -> usize {
        if self.active_view != AppView::MissingOptionalDeps {
            return 0;
        }

        self.missing_optional_deps
            .iter()
            .map(|dep| reason_signal_width(&optional_reason_signals(dep)))
            .max()
            .unwrap_or_default()
    }
}

fn capped_list_pane_width(
    max_width: u16,
    title: &str,
    item_width: usize,
    has_selection: bool,
) -> u16 {
    let highlight_symbol_width = if has_selection {
        LIST_HIGHLIGHT_SYMBOL_WIDTH
    } else {
        0
    };
    let item_pane_width = item_width.saturating_add(usize::from(
        LIST_BLOCK_BORDER_WIDTH + highlight_symbol_width,
    ));
    let title_pane_width = Line::from(title.to_owned())
        .width()
        .saturating_add(usize::from(LIST_BLOCK_BORDER_WIDTH));

    item_pane_width.max(title_pane_width).min(max_width.into()) as u16
}

fn list_item_width(list_area: Rect, has_selection: bool) -> usize {
    let highlight_symbol_width = if has_selection {
        LIST_HIGHLIGHT_SYMBOL_WIDTH
    } else {
        0
    };

    list_area
        .width
        .saturating_sub(LIST_BLOCK_BORDER_WIDTH + highlight_symbol_width)
        .into()
}

fn package_list_item(package: &PackageInfo) -> ListItem<'static> {
    ListItem::new(package_list_line(package))
}

fn package_list_line(package: &PackageInfo) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            package.name.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            package.version.clone(),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

fn package_list_required_width(package: &PackageInfo) -> usize {
    package_list_line(package).width()
}

fn package_details(package: &PackageInfo) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                package.name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                package.version.clone(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
        Line::from(
            package
                .description
                .clone()
                .unwrap_or_else(|| "No package description available.".to_owned()),
        ),
    ];

    if package.optional_deps.is_empty() {
        return lines;
    }

    lines.push(Line::from(""));
    lines.push(Line::from("Optional deps:"));

    for dep in &package.optional_deps {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(dep.name.clone(), Style::default().fg(Color::Yellow)),
            Span::raw(": "),
            Span::from(dep.reason()),
            Span::raw("  "),
        ];
        spans.extend(installed_status_spans(dep));
        lines.push(Line::from(spans));
    }

    lines
}

fn installed_status_spans(dep: &OptionalDep) -> Vec<Span<'static>> {
    match &dep.installed_package {
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

fn missing_optional_dep_list_item(
    dep: &MissingOptionalDep,
    checked: bool,
    range_selected: bool,
    row_width: usize,
    signal_column_width: usize,
) -> ListItem<'static> {
    let item = ListItem::new(missing_optional_dep_list_line(
        dep,
        checked,
        row_width,
        signal_column_width,
    ));
    if range_selected {
        item.style(Style::default().bg(Color::DarkGray))
    } else {
        item
    }
}

fn missing_optional_dep_list_line(
    dep: &MissingOptionalDep,
    checked: bool,
    row_width: usize,
    signal_column_width: usize,
) -> Line<'static> {
    let (mut left_spans, right_spans) =
        missing_optional_dep_list_spans(dep, checked, signal_column_width);
    let left_width = Line::from(left_spans.clone()).width();
    let right_width = Line::from(right_spans.clone()).width();
    let spacer_width = if left_width + right_width < row_width {
        row_width - left_width - right_width
    } else {
        2
    };

    left_spans.push(Span::raw(" ".repeat(spacer_width)));
    left_spans.extend(right_spans);
    Line::from(left_spans)
}

fn missing_optional_dep_list_required_width(
    dep: &MissingOptionalDep,
    signal_column_width: usize,
) -> usize {
    let (left_spans, right_spans) =
        missing_optional_dep_list_spans(dep, false, signal_column_width);

    Line::from(left_spans).width() + 2 + Line::from(right_spans).width()
}

fn missing_optional_dep_list_spans(
    dep: &MissingOptionalDep,
    checked: bool,
    signal_column_width: usize,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let checkbox = if checked { "[x]" } else { "[ ]" };
    let signals = optional_reason_signals(dep);
    let mut left_spans = vec![
        Span::styled(checkbox, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(dep.name.clone(), missing_optional_dep_name_style(&signals)),
    ];

    if let Some(version) = &dep.version {
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            version.clone(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let mut right_spans = vec![Span::styled(
        format!("wanted by {}", dep.wanted_by.len()),
        Style::default().fg(Color::DarkGray),
    )];
    right_spans.extend(reserved_reason_signal_spans(&signals, signal_column_width));

    (left_spans, right_spans)
}

fn missing_optional_dep_details(dep: &MissingOptionalDep) -> Vec<Line<'static>> {
    let mut title = vec![Span::styled(
        dep.name.clone(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    if let Some(version) = &dep.version {
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
            dep.description
                .clone()
                .unwrap_or_else(|| "No package description available.".to_owned()),
        ),
        Line::from(""),
        Line::from("Wanted by:"),
    ];

    let signals = optional_reason_signals(dep);
    if !signals.is_empty() {
        let mut signal_spans = vec![Span::styled(
            "Signals:",
            Style::default().fg(Color::DarkGray),
        )];
        signal_spans.extend(full_reason_signal_spans(&signals));
        lines.push(Line::from(""));
        lines.push(Line::from(signal_spans));
        lines.push(Line::from(""));
    }

    for requester in &dep.wanted_by {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(
                requester.package_name.clone(),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(": "),
            Span::from(requester.reason.clone()),
        ];
        spans.extend(full_reason_signal_spans(&reason_signals(&requester.reason)));
        lines.push(Line::from(spans));
    }

    lines
}

fn optional_reason_signals(dep: &MissingOptionalDep) -> Vec<OptionalReasonSignal> {
    collect_reason_signals(
        dep.wanted_by
            .iter()
            .flat_map(|requester| reason_signals(&requester.reason)),
    )
}

fn reason_signals(reason: &str) -> Vec<OptionalReasonSignal> {
    let mut signals = Vec::new();

    if contains_any_keyword(reason, BENEFIT_REASON_KEYWORDS) {
        signals.push(OptionalReasonSignal::Benefit);
    }
    if contains_any_keyword(reason, ALTERNATIVE_REASON_KEYWORDS) {
        signals.push(OptionalReasonSignal::Alternative);
    }

    signals
}

fn collect_reason_signals(
    signals: impl IntoIterator<Item = OptionalReasonSignal>,
) -> Vec<OptionalReasonSignal> {
    let mut collected = Vec::new();

    for signal in signals {
        if !collected.contains(&signal) {
            collected.push(signal);
        }
    }

    collected
}

fn contains_any_keyword(reason: &str, keywords: &[&str]) -> bool {
    keywords
        .iter()
        .any(|keyword| contains_keyword(reason, keyword))
}

fn contains_keyword(reason: &str, keyword: &str) -> bool {
    let reason = reason.to_lowercase();
    let keyword = keyword.to_lowercase();

    reason.match_indices(&keyword).any(|(start, _)| {
        let end = start + keyword.len();
        let before_is_boundary = reason[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_keyword_character(character));
        let after_is_boundary = reason[end..]
            .chars()
            .next()
            .is_none_or(|character| !is_keyword_character(character));

        before_is_boundary && after_is_boundary
    })
}

fn is_keyword_character(character: char) -> bool {
    character.is_alphanumeric()
}

fn missing_optional_dep_name_style(signals: &[OptionalReasonSignal]) -> Style {
    let color = match primary_reason_signal(signals) {
        Some(OptionalReasonSignal::Benefit) => Color::Green,
        Some(OptionalReasonSignal::Alternative) => Color::Magenta,
        None => Color::White,
    };

    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn primary_reason_signal(signals: &[OptionalReasonSignal]) -> Option<OptionalReasonSignal> {
    if signals.contains(&OptionalReasonSignal::Benefit) {
        Some(OptionalReasonSignal::Benefit)
    } else if signals.contains(&OptionalReasonSignal::Alternative) {
        Some(OptionalReasonSignal::Alternative)
    } else {
        None
    }
}

fn compact_reason_signal_spans(signals: &[OptionalReasonSignal]) -> Vec<Span<'static>> {
    let Some(signal) = primary_reason_signal(signals) else {
        return Vec::new();
    };

    vec![
        Span::raw(" "),
        Span::styled(
            format!("[{}]", signal.short_label()),
            signal.style().add_modifier(Modifier::BOLD),
        ),
    ]
}

fn full_reason_signal_spans(signals: &[OptionalReasonSignal]) -> Vec<Span<'static>> {
    let Some(signal) = primary_reason_signal(signals) else {
        return Vec::new();
    };

    vec![
        Span::raw(" "),
        Span::styled(
            format!("[{}]", signal.full_label()),
            signal.style().add_modifier(Modifier::BOLD),
        ),
    ]
}

fn reserved_reason_signal_spans(
    signals: &[OptionalReasonSignal],
    reserved_width: usize,
) -> Vec<Span<'static>> {
    let mut spans = compact_reason_signal_spans(signals);
    let width = Line::from(spans.clone()).width();
    if width < reserved_width {
        spans.push(Span::raw(" ".repeat(reserved_width - width)));
    }

    spans
}

fn reason_signal_width(signals: &[OptionalReasonSignal]) -> usize {
    Line::from(compact_reason_signal_spans(signals)).width()
}

impl OptionalReasonSignal {
    fn short_label(self) -> &'static str {
        match self {
            OptionalReasonSignal::Benefit => "b",
            OptionalReasonSignal::Alternative => "a",
        }
    }

    fn full_label(self) -> &'static str {
        match self {
            OptionalReasonSignal::Benefit => "benefit",
            OptionalReasonSignal::Alternative => "alternative",
        }
    }

    fn style(self) -> Style {
        match self {
            OptionalReasonSignal::Benefit => Style::default().fg(Color::Green),
            OptionalReasonSignal::Alternative => Style::default().fg(Color::Magenta),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InstalledPackage, OptionalDepRequester};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

        let rendered = package_details(&package)
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

        let rendered = package_details(&package)
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

        let rendered = package_details(&package)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.contains(&"  mysql: database support  not installed".to_owned()));
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

        let rendered = missing_optional_dep_details(&dep)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.contains(&"SQL database server".to_owned()));
        assert!(rendered.contains(&"  alpha: database support".to_owned()));
        assert!(rendered.contains(&"  beta: mysql backend".to_owned()));
    }

    #[test]
    fn missing_optional_dep_details_include_reason_signal_tags() {
        let dep = MissingOptionalDep {
            name: "ripgrep-all".to_owned(),
            version: None,
            description: None,
            wanted_by: vec![
                OptionalDepRequester {
                    package_name: "alpha".to_owned(),
                    reason: "faster archive scanning".to_owned(),
                },
                OptionalDepRequester {
                    package_name: "beta".to_owned(),
                    reason: "drop-in replacement backend".to_owned(),
                },
            ],
        };

        let rendered = missing_optional_dep_details(&dep)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.contains(&"Signals: [benefit]".to_owned()));
        assert!(rendered.contains(&"  alpha: faster archive scanning [benefit]".to_owned()));
        assert!(rendered.contains(&"  beta: drop-in replacement backend [alternative]".to_owned()));
    }

    #[test]
    fn reason_signal_display_prefers_benefit_over_alternative() {
        let rendered = Line::from(compact_reason_signal_spans(&[
            OptionalReasonSignal::Alternative,
            OptionalReasonSignal::Benefit,
        ]))
        .to_string();

        assert_eq!(rendered, " [b]");
    }

    #[test]
    fn missing_optional_dep_list_aligns_wanted_by_and_signals_to_the_right() {
        let dep = MissingOptionalDep {
            name: "ripgrep-all".to_owned(),
            version: Some("1.0.0-1".to_owned()),
            description: None,
            wanted_by: vec![OptionalDepRequester {
                package_name: "alpha".to_owned(),
                reason: "faster archive scanning".to_owned(),
            }],
        };

        let rendered = missing_optional_dep_list_line(&dep, false, 60, 4).to_string();

        assert_eq!(rendered.len(), 60);
        assert!(rendered.starts_with("[ ] ripgrep-all  1.0.0-1"));
        assert!(rendered.ends_with("wanted by 1 [b]"));
    }

    #[test]
    fn missing_optional_dep_list_reserves_signal_space_when_absent() {
        let dep = MissingOptionalDep {
            name: "tk".to_owned(),
            version: Some("8.6.16-1".to_owned()),
            description: None,
            wanted_by: vec![OptionalDepRequester {
                package_name: "alpha".to_owned(),
                reason: "database support".to_owned(),
            }],
        };

        let rendered = missing_optional_dep_list_line(&dep, false, 60, 4).to_string();

        assert_eq!(rendered.len(), 60);
        assert!(rendered.starts_with("[ ] tk  8.6.16-1"));
        assert!(rendered.ends_with("wanted by 1    "));
    }

    #[test]
    fn list_pane_shrinks_to_the_needed_package_width() {
        let mut app = App::new(vec![
            PackageInfo {
                name: "a".to_owned(),
                version: "1".to_owned(),
                description: None,
                optional_deps: Vec::new(),
            },
            PackageInfo {
                name: "a-really-long-package-name".to_owned(),
                version: "2".to_owned(),
                description: None,
                optional_deps: Vec::new(),
            },
        ]);
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| frame.render_widget(&mut app, frame.area()))
            .unwrap();

        assert_eq!(app.last_list_area.unwrap().width, 33);
    }

    #[test]
    fn list_pane_keeps_the_old_percentage_width_as_a_cap() {
        let mut app = App::new(vec![PackageInfo {
            name: "an-extremely-long-package-name-that-would-overflow-the-old-pane-width"
                .to_owned(),
            version: "1".to_owned(),
            description: None,
            optional_deps: Vec::new(),
        }]);
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| frame.render_widget(&mut app, frame.area()))
            .unwrap();

        assert_eq!(app.last_list_area.unwrap().width, 40);
    }

    #[test]
    fn reason_signal_matching_uses_keyword_boundaries() {
        assert_eq!(
            reason_signals("use a safer sandbox"),
            vec![OptionalReasonSignal::Benefit]
        );
        assert_eq!(
            reason_signals("more memory-efficient backend"),
            vec![OptionalReasonSignal::Benefit]
        );
        assert_eq!(
            reason_signals("optimized and quicker search"),
            vec![OptionalReasonSignal::Benefit]
        );
        assert_eq!(
            reason_signals("simd-optimized decoding"),
            vec![OptionalReasonSignal::Benefit]
        );
        assert_eq!(
            reason_signals("gpu-accelerated rendering"),
            vec![OptionalReasonSignal::Benefit]
        );
        assert_eq!(
            reason_signals("alternative implementation"),
            vec![OptionalReasonSignal::Alternative]
        );
        assert_eq!(
            reason_signals("replacing default backend"),
            vec![OptionalReasonSignal::Alternative]
        );
        assert_eq!(
            reason_signals("substitution for legacy parser"),
            vec![OptionalReasonSignal::Alternative]
        );
        assert_eq!(
            reason_signals("substitutes the bundled renderer"),
            vec![OptionalReasonSignal::Alternative]
        );
        assert_eq!(
            reason_signals("drop in implementation"),
            vec![OptionalReasonSignal::Alternative]
        );
        assert_eq!(
            reason_signals("dropin implementation"),
            vec![OptionalReasonSignal::Alternative]
        );
        assert!(reason_signals("unsafe compatibility mode").is_empty());
        assert!(reason_signals("unoptimized build").is_empty());
        assert!(reason_signals("irreplaceable package").is_empty());
        assert!(reason_signals("dropinside helper").is_empty());
        assert!(reason_signals("substitutability check").is_empty());
    }

    #[test]
    fn details_pane_wraps_long_descriptions() {
        let mut app = App::with_missing_optional_deps(
            vec![PackageInfo {
                name: "example".to_owned(),
                version: "1.0.0".to_owned(),
                description: Some(
                    "This description is intentionally long enough to wrap inside a narrow details pane."
                        .to_owned(),
                ),
                optional_deps: Vec::new(),
            }],
            Vec::new(),
        );
        let backend = TestBackend::new(50, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| frame.render_widget(&mut app, frame.area()))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("intentionally long"));
        assert!(rendered.contains("inside a narrow"));
    }
}

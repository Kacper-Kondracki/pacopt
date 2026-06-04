use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eyre::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    pub count: i64,
    pub running: bool,
}

impl Model {
    pub fn new() -> Self {
        Self {
            count: 0,
            running: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Increment,
    AutoIncrement,
    Decrement,
    Reset,
    Quit,
    NoOp,
}

pub fn update(model: &mut Model, msg: Msg) {
    match msg {
        Msg::Increment | Msg::AutoIncrement => model.count += 1,
        Msg::Decrement => model.count -= 1,
        Msg::Reset => model.count = 0,
        Msg::Quit => model.running = false,
        Msg::NoOp => {}
    }
}

pub fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut model = Model::new();
    let rx = start_runtime();

    while model.running {
        terminal.draw(|frame| view(frame, &model))?;
        let msg = rx.recv()?;
        update(&mut model, msg);
    }

    Ok(())
}

pub fn view(frame: &mut Frame<'_>, model: &Model) {
    let area = frame.area();
    let [_, app_area, _] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(9),
            Constraint::Min(0),
        ])
        .areas(area);

    let [_, app_area, _] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(48),
            Constraint::Min(0),
        ])
        .areas(app_area);

    let counter = Span::styled(
        model.count.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let content = vec![
        Line::from(""),
        Line::from(vec!["Count: ".into(), counter]),
        Line::from(""),
        Line::from("Background thread increments every second"),
        Line::from("Press + to increment, - to decrement, 0 to reset"),
        Line::from("Press q or Esc to quit"),
    ];

    let widget = Paragraph::new(content)
        .block(
            Block::default()
                .title(" Counter ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Gray));

    frame.render_widget(widget, app_area);
}

fn start_runtime() -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();

    spawn_input_thread(tx.clone());
    spawn_tick_thread(tx);

    rx
}

fn spawn_input_thread(tx: Sender<Msg>) {
    thread::spawn(move || {
        while let Ok(msg) = read_input_msg() {
            let should_quit = msg == Msg::Quit;

            if tx.send(msg).is_err() || should_quit {
                break;
            }
        }
    });
}

fn spawn_tick_thread(tx: Sender<Msg>) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(1));

            if tx.send(Msg::AutoIncrement).is_err() {
                break;
            }
        }
    });
}

fn read_input_msg() -> Result<Msg> {
    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => return Ok(key_to_msg(key)),
            _ => {}
        }
    }
}

fn key_to_msg(key: KeyEvent) -> Msg {
    match key.code {
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up => Msg::Increment,
        KeyCode::Char('-') | KeyCode::Down => Msg::Decrement,
        KeyCode::Char('0') => Msg::Reset,
        KeyCode::Char('q') | KeyCode::Esc => Msg::Quit,
        _ => Msg::NoOp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_handles_counter_messages() {
        let mut model = Model::new();

        update(&mut model, Msg::Increment);
        update(&mut model, Msg::Increment);
        update(&mut model, Msg::Decrement);
        assert_eq!(model.count, 1);

        update(&mut model, Msg::Reset);
        assert_eq!(model.count, 0);
    }

    #[test]
    fn quit_stops_the_app() {
        let mut model = Model::new();

        update(&mut model, Msg::Quit);

        assert!(!model.running);
    }
}

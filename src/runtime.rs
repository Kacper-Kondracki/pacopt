//! Runtime event types and input polling helpers.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use eyre::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseEvent,
};
use ratatui::crossterm::execute;

/// Input or control message consumed by the application state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// Keyboard input from crossterm.
    Key(KeyEvent),
    /// Mouse input from crossterm.
    Mouse(MouseEvent),
    /// Move selection down in the active list.
    SelectNext,
    /// Move selection up in the active list.
    SelectPrevious,
    /// Stop the application loop.
    Quit,
    /// Intentionally do nothing.
    NoOp,
}

/// Starts background input polling for the terminal application.
pub(crate) struct Runtime;

/// Enables mouse capture and disables it again when dropped.
pub(crate) struct MouseCaptureGuard;

impl MouseCaptureGuard {
    /// Enables crossterm mouse capture for the current terminal session.
    pub(crate) fn enable() -> Result<Self> {
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
    /// Starts the input thread and returns a receiver for application messages.
    pub(crate) fn start() -> Receiver<Msg> {
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

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use eyre::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseEvent,
};
use ratatui::crossterm::execute;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    SelectNext,
    SelectPrevious,
    Quit,
    NoOp,
}

pub(crate) struct Runtime;

pub(crate) struct MouseCaptureGuard;

impl MouseCaptureGuard {
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

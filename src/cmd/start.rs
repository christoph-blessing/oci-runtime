use std::{
    error::Error,
    fmt::Display,
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
};

use crate::state::{State, StateError, load, persist};

pub fn run(id: &str) -> Result<(), StartError> {
    let created = match load(id)? {
        State::Created(s) => s,
        other => return Err(StartError::NotCreated(other.as_string())),
    };
    send_start_signal(&created.internal.start_signal)?;
    let running = created.start();
    persist(&running.into())?;
    Ok(())
}

fn send_start_signal(path: &Path) -> Result<(), StartError> {
    let mut start_fifo = OpenOptions::new().write(true).open(path)?;
    let mut buffer = [0u8; 1];
    start_fifo.write(&mut buffer)?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[derive(Debug)]
pub enum StartError {
    NotCreated(String),
    State(StateError),
    Io(io::Error),
}

impl From<StateError> for StartError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<io::Error> for StartError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCreated(s) => write!(f, "cannot start container in state {}", s),
            Self::State(_) => write!(f, "state error during start"),
            Self::Io(_) => write!(f, "i/o error during start"),
        }
    }
}

impl Error for StartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotCreated(_) => None,
            Self::State(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

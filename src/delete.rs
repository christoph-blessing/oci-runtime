use std::{error::Error, fmt::Display, io};

use crate::state::{State, StateError};

pub fn run(id: &str) -> Result<(), DeleteError> {
    match crate::state::load(id)? {
        State::Stopped(_) => {}
        other => return Err(DeleteError::NotStopped(other.as_string())),
    };
    let dir = crate::state::state_dir(id);
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[derive(Debug)]
pub enum DeleteError {
    NotStopped(String),
    State(StateError),
    Io(io::Error),
}

impl From<StateError> for DeleteError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<io::Error> for DeleteError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl Display for DeleteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStopped(s) => write!(f, "cannot delete container in state {}", s),
            Self::State(_) => write!(f, "state error during delete"),
            Self::Io(_) => write!(f, "i/o error during delete"),
        }
    }
}

impl Error for DeleteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotStopped(_) => None,
            Self::State(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

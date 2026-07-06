use std::{error::Error, fmt::Display, str::FromStr};

use nix::{sys::signal::Signal, unistd::Pid};

use crate::state::{State, StateError};

pub fn run(id: &str, raw_signal: &str) -> Result<(), KillError> {
    let raw_pid = match crate::state::load(id)? {
        State::Created(c) => c.pid,
        State::Running(r) => r.pid,
        other => return Err(KillError::NotKillable(other.as_string())),
    };
    let pid = Pid::from_raw(raw_pid);
    let signal = Signal::from_str(raw_signal)?;
    nix::sys::signal::kill(pid, signal)?;
    Ok(())
}

#[derive(Debug)]
pub enum KillError {
    InvalidSignal,
    NotKillable(String),
    Syscall(nix::Error),
    State(StateError),
}

impl From<nix::Error> for KillError {
    fn from(value: nix::Error) -> Self {
        match value {
            nix::Error::EINVAL => Self::InvalidSignal,
            other => Self::Syscall(other),
        }
    }
}

impl From<StateError> for KillError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl Display for KillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignal => write!(f, "invalid signal"),
            Self::NotKillable(s) => write!(f, "cannot kill container in state {}", s),
            Self::Syscall(_) => write!(f, "syscall error during kill"),
            Self::State(_) => write!(f, "state error during kill"),
        }
    }
}

impl Error for KillError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidSignal | Self::NotKillable(_) => None,
            Self::Syscall(e) => Some(e),
            Self::State(e) => Some(e),
        }
    }
}

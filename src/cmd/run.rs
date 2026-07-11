use std::{error::Error, fmt::Display, path::Path};

use nix::unistd::Pid;

use crate::{
    cmd::{create::CreateError, delete::DeleteError, start::StartError},
    state::{ExitReason, State, StateError},
};

pub fn run(id: &str, bundle: &Path) -> Result<ExitReason, RunError> {
    let shim_pid = crate::cmd::create::run(id, bundle)?;
    crate::cmd::start::run(id)?;

    nix::sys::wait::waitpid(Pid::from_raw(shim_pid as i32), None)?;

    let stopped = match crate::state::load(id)? {
        State::Stopped(stopped) => stopped,
        other => return Err(RunError::UnexpectedState(other.as_string())),
    };

    crate::cmd::delete::run(id)?;
    Ok(stopped.internal.exit_reason)
}

#[derive(Debug)]
pub enum RunError {
    Create(CreateError),
    Start(StartError),
    Wait(nix::Error),
    State(StateError),
    UnexpectedState(String),
    Delete(DeleteError),
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Create(e) => Some(e),
            Self::Start(e) => Some(e),
            Self::Wait(e) => Some(e),
            Self::State(e) => Some(e),
            Self::UnexpectedState(_) => None,
            Self::Delete(e) => Some(e),
        }
    }
}

impl Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create(_) => write!(f, "failed to create container"),
            Self::Start(_) => write!(f, "failed to start container"),
            Self::Wait(_) => write!(f, "failed to wait for container"),
            Self::State(_) => write!(f, "failed to retrieve container state"),
            Self::UnexpectedState(state) => {
                write!(f, "unexpected state after supervision ended: {}", state)
            }
            Self::Delete(_) => write!(f, "failed to delete container"),
        }
    }
}

impl From<CreateError> for RunError {
    fn from(value: CreateError) -> Self {
        Self::Create(value)
    }
}

impl From<StartError> for RunError {
    fn from(value: StartError) -> Self {
        Self::Start(value)
    }
}

impl From<nix::Error> for RunError {
    fn from(value: nix::Error) -> Self {
        Self::Wait(value)
    }
}

impl From<StateError> for RunError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<DeleteError> for RunError {
    fn from(value: DeleteError) -> Self {
        Self::Delete(value)
    }
}

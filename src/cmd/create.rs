use crate::{config::error::ConfigError, state::StateError};
use nix::fcntl::{FcntlArg, FdFlag};
use std::{
    error::Error,
    fmt::Display,
    io::{self, PipeReader, Read},
    os::fd::AsRawFd,
    path::Path,
    process::Command,
};

pub const READY: u8 = 0;
pub const ALREADY_EXISTS: u8 = 1;
pub const CONFIG_NOT_FOUND: u8 = 2;
pub const CONFIG_PARSE: u8 = 3;
pub const SYSCALL: u8 = 4;
pub const IO: u8 = 5;
pub const CHILD_REPORTED: u8 = 6;
pub const STATE: u8 = 7;
pub const CONFIG: u8 = 8;
pub const VALIDATION: u8 = 9;

#[derive(Debug)]
pub enum CreateError {
    State(StateError),
    Config(ConfigError),
    Io(io::Error),
    Syscall(nix::Error),
    ShimExitedEarly,
    ShimReported(ShimReportedError),
}

impl From<StateError> for CreateError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<io::Error> for CreateError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<nix::Error> for CreateError {
    fn from(value: nix::Error) -> Self {
        Self::Syscall(value)
    }
}

impl Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShimExitedEarly => write!(f, "shim exited early"),
            Self::ShimReported(_) => write!(f, "shim reported error"),
            Self::Io(_) => write!(f, "i/o error during creation"),
            Self::Syscall(_) => write!(f, "syscall error during creation"),
            Self::Config(_) => write!(f, "config error during creation"),
            Self::State(_) => write!(f, "state error during creation"),
        }
    }
}

impl Error for CreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(e) => Some(e),
            Self::Config(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Syscall(e) => Some(e),
            Self::ShimExitedEarly => None,
            Self::ShimReported(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum ShimReportedError {
    AlreadyExists,
    ConfigNotFound,
    ConfigParse,
    Syscall,
    Io,
    ChildReported,
    State,
    Config,
    Validation,
    UnexpectedExit,
}

impl ShimReportedError {
    fn from_signal(byte: u8) -> Self {
        match byte {
            ALREADY_EXISTS => Self::AlreadyExists,
            CONFIG_NOT_FOUND => Self::ConfigNotFound,
            CONFIG_PARSE => Self::ConfigParse,
            SYSCALL => Self::Syscall,
            IO => Self::Io,
            CHILD_REPORTED => Self::ChildReported,
            STATE => Self::State,
            CONFIG => Self::Config,
            VALIDATION => Self::Validation,
            _ => Self::UnexpectedExit,
        }
    }
}

impl Display for ShimReportedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists => write!(f, "container already exists"),
            Self::ConfigNotFound => write!(f, "config not found"),
            Self::ConfigParse => write!(f, "malformed config"),
            Self::Syscall => write!(f, "syscall error"),
            Self::Io => write!(f, "i/o error"),
            Self::ChildReported => write!(f, "child reported error"),
            Self::State => write!(f, "state error"),
            Self::Config => write!(f, "config error"),
            Self::Validation => write!(f, "validation error"),
            Self::UnexpectedExit => write!(f, "unexpected exit"),
        }
    }
}

impl Error for ShimReportedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AlreadyExists
            | Self::ConfigNotFound
            | Self::ConfigParse
            | Self::Syscall
            | Self::Io
            | Self::Validation
            | Self::State
            | Self::ChildReported
            | Self::Config
            | Self::UnexpectedExit => None,
        }
    }
}

pub fn run(container_id: &str, bundle_path: &Path) -> Result<(), CreateError> {
    if crate::state::exists(container_id) {
        return Err(CreateError::State(StateError::AlreadyExists(
            container_id.to_string(),
        )));
    }

    let config_path = bundle_path.join("config.json");
    if !config_path.exists() {
        return Err(CreateError::Config(ConfigError::NotFound(config_path)));
    }

    let (recv_shim_done, send_shim_done) = std::io::pipe()?;
    nix::fcntl::fcntl(&send_shim_done, FcntlArg::F_SETFD(FdFlag::empty()))?;

    let program = std::env::current_exe()?;
    Command::new(program)
        .arg("__shim")
        .arg(container_id)
        .arg(bundle_path)
        .arg(send_shim_done.as_raw_fd().to_string())
        .spawn()?;
    drop(send_shim_done);

    wait_for_shim(recv_shim_done)?;
    Ok(())
}

fn wait_for_shim(mut reader: PipeReader) -> Result<(), CreateError> {
    let mut buf = [0u8; 1];
    let n = reader.read(&mut buf)?;

    if n == 0 {
        return Err(CreateError::ShimExitedEarly);
    }

    match buf[0] {
        READY => {}
        other => {
            return Err(CreateError::ShimReported(ShimReportedError::from_signal(
                other,
            )));
        }
    }
    Ok(())
}

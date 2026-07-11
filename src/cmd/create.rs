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
pub const STATE: u8 = 6;
pub const CONFIG: u8 = 7;
pub const VALIDATION: u8 = 8;
pub const CHILD_EXEC_NOT_FOUND: u8 = 9;
pub const CHILD_SYSCALL: u8 = 10;
pub const CHILD_IO: u8 = 11;
pub const CHILD_NUL_BYTE: u8 = 12;
pub const CHILD_CAPS: u8 = 13;
pub const CHILD_UNEXPECTED: u8 = 14;

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
    ChildReported(ChildReportedError),
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
            CHILD_EXEC_NOT_FOUND | CHILD_SYSCALL | CHILD_IO | CHILD_NUL_BYTE | CHILD_CAPS
            | CHILD_UNEXPECTED => Self::ChildReported(ChildReportedError::from_signal(byte)),
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
            Self::ChildReported(_) => write!(f, "child reported error"),
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
            Self::ChildReported(e) => Some(e),
            Self::AlreadyExists
            | Self::ConfigNotFound
            | Self::ConfigParse
            | Self::Syscall
            | Self::Io
            | Self::Validation
            | Self::State
            | Self::Config
            | Self::UnexpectedExit => None,
        }
    }
}

#[derive(Debug)]
pub enum ChildReportedError {
    ExecutableNotFound,
    Syscall,
    Io,
    NulByte,
    Capabilities,
    ShimUnexpectedSignal,
    UnexpectedSignal,
}

impl ChildReportedError {
    fn from_signal(byte: u8) -> Self {
        match byte {
            CHILD_EXEC_NOT_FOUND => Self::ExecutableNotFound,
            CHILD_SYSCALL => Self::Syscall,
            CHILD_IO => Self::Io,
            CHILD_NUL_BYTE => Self::NulByte,
            CHILD_CAPS => Self::Capabilities,
            CHILD_UNEXPECTED => Self::ShimUnexpectedSignal,
            _ => Self::UnexpectedSignal,
        }
    }
}

impl Error for ChildReportedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ExecutableNotFound
            | Self::Syscall
            | Self::Io
            | Self::NulByte
            | Self::Capabilities
            | Self::ShimUnexpectedSignal
            | Self::UnexpectedSignal => None,
        }
    }
}

impl Display for ChildReportedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutableNotFound => write!(f, "executable not found"),
            Self::Syscall => write!(f, "syscall error during child setup"),
            Self::Io => write!(f, "i/o error during child setup"),
            Self::NulByte => write!(f, "nul byte error during child setup"),
            Self::Capabilities => write!(f, "capabilities error during child setup"),
            Self::ShimUnexpectedSignal => write!(f, "shim received unexpected signal from child"),
            Self::UnexpectedSignal => write!(f, "received unexpected signal from shim"),
        }
    }
}

pub fn run(container_id: &str, bundle_path: &Path) -> Result<u32, CreateError> {
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
    let child = Command::new(program)
        .arg("__shim")
        .arg(container_id)
        .arg(bundle_path)
        .arg(send_shim_done.as_raw_fd().to_string())
        .spawn()?;
    drop(send_shim_done);

    wait_for_shim(recv_shim_done)?;
    Ok(child.id())
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

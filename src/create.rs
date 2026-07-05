use crate::{config::error::ConfigError, state::StateError};
use nix::fcntl::{FcntlArg, FdFlag};
use std::{
    io::{self, PipeReader, Read},
    os::fd::AsRawFd,
    path::Path,
    process::Command,
};

pub const READY: u8 = 0;
pub const ALREADY_EXISTS: u8 = 1;
pub const CONFIG_NOT_FOUND: u8 = 2;
pub const CONFIG_PARSE: u8 = 3;

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

#[derive(Debug)]
pub enum ShimReportedError {
    AlreadyExists,
    ConfigNotFound,
    ConfigParse,
    UnexpectedExit,
}

impl ShimReportedError {
    fn from_signal(byte: u8) -> Self {
        match byte {
            ALREADY_EXISTS => Self::AlreadyExists,
            CONFIG_NOT_FOUND => Self::ConfigNotFound,
            CONFIG_PARSE => Self::ConfigParse,
            _ => Self::UnexpectedExit,
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

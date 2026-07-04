use crate::state::StateError;
use nix::fcntl::{FcntlArg, FdFlag};
use std::{
    io::{self, PipeReader, Read},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::Command,
};

pub const READY: u8 = 0;
pub const ALREADY_EXISTS: u8 = 1;
pub const CONFIG_NOT_FOUND: u8 = 2;

#[derive(Debug)]
pub enum CreateError {
    State(StateError),
    Io(io::Error),
    Syscall(nix::Error),
    AlreadyExists(String),
    ConfigNotFound(PathBuf),
    ShimExitedEarly,
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

pub fn run(container_id: &str, bundle_path: &Path) -> Result<(), CreateError> {
    if crate::state::exists(container_id) {
        return Err(CreateError::AlreadyExists(container_id.to_string()));
    }

    let config_path = bundle_path.join("config.json");
    if !config_path.exists() {
        return Err(CreateError::ConfigNotFound(config_path));
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

    match wait_for_shim(recv_shim_done)? {
        ShimStatus::Ready => return Ok(()),
        ShimStatus::AlreadyExists => {
            return Err(CreateError::AlreadyExists(container_id.to_string()));
        }
        ShimStatus::ConfigNotFound => return Err(CreateError::ConfigNotFound(config_path)),
        ShimStatus::EarlyExit => return Err(CreateError::ShimExitedEarly),
    }
}

fn wait_for_shim(mut reader: PipeReader) -> Result<ShimStatus, io::Error> {
    let mut buf = [0u8; 1];
    let n = reader.read(&mut buf)?;

    if n == 0 {
        return Ok(ShimStatus::EarlyExit);
    }

    let status = match buf[0] {
        READY => ShimStatus::Ready,
        ALREADY_EXISTS => ShimStatus::AlreadyExists,
        CONFIG_NOT_FOUND => ShimStatus::ConfigNotFound,
        other => panic!("unexpected signal from shim: {}", other),
    };
    Ok(status)
}

enum ShimStatus {
    Ready,
    EarlyExit,
    AlreadyExists,
    ConfigNotFound,
}

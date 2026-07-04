use crate::state::StateError;
use nix::fcntl::{FcntlArg, FdFlag};
use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug)]
pub enum CreateError {
    State(StateError),
    Io(io::Error),
    Syscall(nix::Error),
    AlreadyExists(String),
    ConfigNotFound(PathBuf),
    ShimExitedEarly,
    ShimReportedFailure,
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

    let (mut recv_shim_done, send_shim_done) = std::io::pipe()?;
    nix::fcntl::fcntl(&send_shim_done, FcntlArg::F_SETFD(FdFlag::empty()))?;

    let program = std::env::current_exe()?;
    Command::new(program)
        .arg("__shim")
        .arg(container_id)
        .arg(bundle_path)
        .arg(send_shim_done.as_raw_fd().to_string())
        .spawn()?;
    drop(send_shim_done);

    let mut buffer = [0u8; 1];
    let n = recv_shim_done.read(&mut buffer)?;
    if n == 0 {
        return Err(CreateError::ShimExitedEarly);
    } else {
        if buffer[0] == 0 {
            return Err(CreateError::ShimReportedFailure);
        } else {
            Ok(())
        }
    }
}

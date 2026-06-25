use crate::state::{State, StateError};
use nix::fcntl::{FcntlArg, FdFlag};
use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    path::PathBuf,
    process::Command,
};

pub enum CreateError {
    State(StateError),
    Io(io::Error),
    Syscall(nix::Error),
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

pub fn run(container_id: &String, bundle_path: &PathBuf) -> Result<(), CreateError> {
    State::new(container_id, bundle_path.to_path_buf(), None)?;
    run_shim(container_id)?;
    Ok(())
}

fn run_shim(id: &str) -> Result<(), CreateError> {
    let (mut recv_shim_ready, send_shim_ready) = std::io::pipe()?;

    nix::fcntl::fcntl(&send_shim_ready, FcntlArg::F_SETFD(FdFlag::empty()))?;

    let program = std::env::current_exe()?;
    Command::new(program)
        .arg("__shim")
        .arg(id)
        .arg(send_shim_ready.as_raw_fd().to_string())
        .spawn()?;

    let mut buffer = [0u8; 1];
    recv_shim_ready.read(&mut buffer)?;
    Ok(())
}

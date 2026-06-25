use std::{
    fs::{self, File},
    io::{self, Read},
    os::fd::BorrowedFd,
    path::{Path, PathBuf},
};

use nix::sys::stat::Mode;

use crate::state::{State, StateError};

pub enum ShimError {
    State(StateError),
    Syscall(nix::Error),
    Io(io::Error),
}

impl From<StateError> for ShimError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<nix::Error> for ShimError {
    fn from(value: nix::Error) -> Self {
        Self::Syscall(value)
    }
}

impl From<io::Error> for ShimError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn run(id: &str, ready_fd: i32) -> Result<(), ShimError> {
    let state = State::load(id)?;
    let start_fifo_path = create_start_signal_fifo(&state)?;
    state.finish_setup(42, start_fifo_path.as_path())?;
    send_ready_signal(ready_fd)?;
    recv_start_signal(&start_fifo_path)?;
    Ok(())
}

fn create_start_signal_fifo(state: &State) -> Result<PathBuf, ShimError> {
    let start_fifo_path = state.state_dir()?.join("start.fifo");
    nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU)?;
    Ok(start_fifo_path)
}

fn send_ready_signal(ready_fd: i32) -> Result<(), ShimError> {
    let mut buffer = [0u8; 1];
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &mut buffer)?;
    Ok(())
}

fn recv_start_signal(start_fifo_path: &Path) -> Result<(), ShimError> {
    let mut start_fifo = File::open(&start_fifo_path)?;
    let mut buffer = [0u8; 1];
    start_fifo.read_exact(&mut buffer)?;
    fs::remove_file(&start_fifo_path)?;
    Ok(())
}

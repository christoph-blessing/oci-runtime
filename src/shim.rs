use std::{
    fs::{self, File},
    io::{self, Read},
    os::fd::BorrowedFd,
    path::{Path, PathBuf},
};

use nix::sys::stat::Mode;

use crate::{
    config::{error::ConfigError, validated::Config},
    state::{State, StateError},
};

#[derive(Debug)]
pub enum ShimError {
    State(StateError),
    Config(ConfigError),
    Syscall(nix::Error),
    Io(io::Error),
}

impl From<StateError> for ShimError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<ConfigError> for ShimError {
    fn from(value: ConfigError) -> Self {
        Self::Config(value)
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

pub fn run(id: &str, bundle: &Path, done_fd: i32) -> Result<(), ShimError> {
    let start_fifo_path = match finish_setup(id, bundle) {
        Ok(p) => {
            send_done_signal(done_fd, true)?;
            p
        }
        Err(e) => {
            send_done_signal(done_fd, false)?;
            return Err(e);
        }
    };
    recv_start_signal(&start_fifo_path)?;
    Ok(())
}

fn finish_setup(id: &str, bundle: &Path) -> Result<PathBuf, ShimError> {
    let state = State::new(id, bundle.to_path_buf(), None)?;
    let start_fifo_path = create_start_signal_fifo(&state)?;
    let config = Config::new(bundle)?;
    let pid = clone_container(&config)?;
    state.finish_setup(pid, start_fifo_path.as_path())?;
    Ok(start_fifo_path)
}

fn create_start_signal_fifo(state: &State) -> Result<PathBuf, ShimError> {
    let start_fifo_path = state.state_dir()?.join("start.fifo");
    nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU)?;
    Ok(start_fifo_path)
}

fn send_done_signal(done_fd: i32, is_success: bool) -> Result<(), ShimError> {
    let mut buf;
    if is_success {
        buf = [1u8; 1];
    } else {
        buf = [0u8; 1];
    }
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(done_fd) }, &mut buf)?;
    Ok(())
}

fn recv_start_signal(start_fifo_path: &Path) -> Result<(), ShimError> {
    let mut start_fifo = File::open(&start_fifo_path)?;
    let mut buffer = [0u8; 1];
    start_fifo.read_exact(&mut buffer)?;
    fs::remove_file(&start_fifo_path)?;
    Ok(())
}

fn clone_container(config: &Config) -> Result<i32, ShimError> {
    Ok(42)
}

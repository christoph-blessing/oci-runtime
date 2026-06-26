use std::{
    io,
    os::fd::BorrowedFd,
    path::{Path, PathBuf},
};

use nix::{
    sys::{signal::Signal, stat::Mode, wait::WaitStatus},
    unistd::Pid,
};

use crate::{
    config::{error::ConfigError, validated::Config},
    state::{State, StateError},
};

use child::ChildError;

mod child;

const STACK_SIZE: usize = 1024 * 1024;

const EXIT_OK: i32 = 0;
const EXIT_FIFO: i32 = 4;

#[derive(Debug)]
pub enum ShimError {
    State(StateError),
    Config(ConfigError),
    Syscall(nix::Error),
    Io(io::Error),
    ChildFifo,
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
    let pid = match finish_setup(id, bundle) {
        Ok(p) => {
            send_done_signal(done_fd, true)?;
            p
        }
        Err(e) => {
            send_done_signal(done_fd, false)?;
            return Err(e);
        }
    };
    match nix::sys::wait::waitpid(pid, None)? {
        WaitStatus::Exited(_, code) => match code {
            EXIT_OK => Ok(()),
            EXIT_FIFO => Err(ShimError::ChildFifo),
            other => panic!("unexpected exit code: {}", other),
        },
        other => panic!("unexpected wait status: {:?}", other),
    }
}

fn finish_setup(id: &str, bundle: &Path) -> Result<Pid, ShimError> {
    let state = State::new(id, bundle.to_path_buf(), None)?;
    let guard = StateGuard::new(id);
    let start_fifo_path = create_start_signal_fifo(&state)?;
    let config = Config::new(bundle)?;
    let pid = clone_child(&config, start_fifo_path.as_path())?;
    state.finish_setup(pid, start_fifo_path.as_path())?;
    guard.confirm();
    Ok(pid)
}

struct StateGuard {
    dir: PathBuf,
    confirmed: bool,
}

impl StateGuard {
    fn new(id: &str) -> Self {
        Self {
            dir: crate::state::state_dir(id),
            confirmed: false,
        }
    }

    fn confirm(mut self) {
        self.confirmed = true;
    }
}

impl Drop for StateGuard {
    fn drop(&mut self) {
        if !self.confirmed {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
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

fn create_start_signal_fifo(state: &State) -> Result<PathBuf, ShimError> {
    let start_fifo_path = state.state_dir().join("start.fifo");
    nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU)?;
    Ok(start_fifo_path)
}

fn clone_child(config: &Config, start_fifo_path: &Path) -> Result<Pid, ShimError> {
    let mut stack = vec![0u8; STACK_SIZE];
    let cb = Box::new(|| match child::run(config, start_fifo_path) {
        Ok(_) => 0,
        Err(e) => match e {
            ChildError::Fifo(_) => 4,
        },
    });
    let pid = unsafe {
        nix::sched::clone(
            cb,
            &mut stack,
            config.linux.clone_flags,
            Some(Signal::SIGCHLD as i32),
        )
    }?;
    Ok(pid)
}

use std::path::Path;

use nix::{sys::wait::WaitStatus, unistd::Pid};

use crate::state::{Creating, StateError};

pub mod child;

const EXIT_OK: i32 = 0;
const EXIT_SYSCALL: i32 = 2;
const EXIT_FIFO: i32 = 4;

#[derive(Debug)]
pub enum ShimError {
    State(StateError),
    Syscall(nix::Error),
    ChildSyscall,
    ChildFifo,
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

pub fn run(id: &str, bundle: &Path, done_fd: i32) -> Result<(), ShimError> {
    let creating = Creating::new(id, bundle.to_path_buf(), None)?;
    let created = creating.finish_setup(done_fd)?;

    match nix::sys::wait::waitpid(Pid::from_raw(created.pid), None)? {
        WaitStatus::Exited(_, code) => match code {
            EXIT_OK => Ok(()),
            EXIT_SYSCALL => Err(ShimError::ChildSyscall),
            EXIT_FIFO => Err(ShimError::ChildFifo),
            other => panic!("unexpected exit code: {}", other),
        },
        other => panic!("unexpected wait status: {:?}", other),
    }
}

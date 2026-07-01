use std::{
    io,
    os::fd::{BorrowedFd, IntoRawFd},
    path::{Path, PathBuf},
};

use nix::{
    sys::{signal::Signal, stat::Mode, wait::WaitStatus},
    unistd::Pid,
};

use crate::{
    config::{
        error::ConfigError,
        validated::{Config, IdMappingConfig},
    },
    shim::child::ChildError,
    state::{Creating, StateError, StateGuard, persist, state_dir},
};

pub mod child;

const STACK_SIZE: usize = 1024 * 1024;
const EXIT_OK: i32 = 0;
const EXIT_SYSCALL: i32 = 2;
const EXIT_IO: i32 = 3;

pub fn run(id: &str, bundle: &Path, done_fd: i32) -> Result<(), ShimError> {
    let pid = match setup_child(id, bundle, done_fd) {
        Ok(p) => p,
        Err(e) => {
            return Err(e);
        }
    };

    match nix::sys::wait::waitpid(pid, None)? {
        WaitStatus::Exited(_, code) => match code {
            EXIT_OK => Ok(()),
            EXIT_SYSCALL => Err(ShimError::ChildSyscall),
            EXIT_IO => Err(ShimError::ChildIo),
            other => panic!("unexpected exit code: {}", other),
        },
        other => panic!("unexpected wait status: {:?}", other),
    }
}

fn setup_child(id: &str, bundle: &Path, ready_fd: i32) -> Result<Pid, ShimError> {
    let signal_guard = ReadySignalGuard::new(ready_fd);

    let creating = Creating::new(id, bundle.to_path_buf(), None);
    persist(&creating.clone().into())?;
    let state_guard = StateGuard::new(id);

    let start_fifo_path = create_start_signal_fifo(id)?;
    let config = Config::new(bundle)?;

    let child_ready_pipe = RawPipe::new()?;
    let mappings_ready_pipe = RawPipe::new()?;

    let cb = Box::new(|| {
        match crate::shim::child::run(
            &config,
            mappings_ready_pipe.read_fd,
            mappings_ready_pipe.write_fd,
            child_ready_pipe.read_fd,
            child_ready_pipe.write_fd,
            &start_fifo_path,
        ) {
            Ok(_) => 0,
            Err(e) => match e {
                ChildError::Syscall(e) => 2,
                ChildError::Io(_) => 3,
                ChildError::NulByte(_) => 4,
                ChildError::Capabilities(_) => 5,
                ChildError::ExecutableNotFound => 6,
            },
        }
    });
    let mut stack = vec![0u8; STACK_SIZE];
    let pid = unsafe {
        nix::sched::clone(
            cb,
            &mut stack,
            config.linux.clone_flags,
            Some(Signal::SIGCHLD as i32),
        )
    }?;

    write_id_mappings(pid, &config)?;
    let mut buf = vec![0u8; 1];
    mappings_ready_pipe.write(&mut buf)?;

    let (n, buf) = child_ready_pipe.read()?;
    if n == 0 {
        return Err(ShimError::ChildExitedEarly);
    } else {
        if buf[0] == 0 {
            return Err(ShimError::ChildReportedFailure);
        }
    }

    let created = creating.finish_setup(pid.as_raw(), &start_fifo_path);
    persist(&created.into())?;
    state_guard.confirm();

    signal_guard.confirm();

    Ok(pid)
}

struct ReadySignalGuard {
    fd: i32,
    confirmed: bool,
}

impl ReadySignalGuard {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            confirmed: false,
        }
    }
    fn confirm(mut self) {
        let mut buf = [1u8; 1];
        _ = nix::unistd::write(unsafe { BorrowedFd::borrow_raw(self.fd) }, &mut buf);
        self.confirmed = true;
    }
}

impl Drop for ReadySignalGuard {
    fn drop(&mut self) {
        if !self.confirmed {
            let mut buf = [0u8; 1];
            _ = nix::unistd::write(unsafe { BorrowedFd::borrow_raw(self.fd) }, &mut buf);
        }
    }
}

struct RawPipe {
    read_fd: i32,
    write_fd: i32,
}

impl RawPipe {
    fn new() -> Result<Self, ShimError> {
        let (read, write) = nix::unistd::pipe()?;
        let read_fd = read.into_raw_fd();
        let write_fd = write.into_raw_fd();
        Ok(Self { read_fd, write_fd })
    }

    fn read(&self) -> Result<(usize, [u8; 1]), ShimError> {
        nix::unistd::close(self.write_fd)?;
        let borrowed = unsafe { BorrowedFd::borrow_raw(self.read_fd) };
        let mut buf = [0u8; 1];
        let n = nix::unistd::read(borrowed, &mut buf)?;
        nix::unistd::close(self.read_fd)?;
        Ok((n, buf))
    }

    fn write(&self, buf: &mut [u8]) -> Result<usize, ShimError> {
        nix::unistd::close(self.read_fd)?;
        let borrowed = unsafe { BorrowedFd::borrow_raw(self.write_fd) };
        let n = nix::unistd::write(borrowed, buf)?;
        nix::unistd::close(self.write_fd)?;
        Ok(n)
    }
}

fn create_start_signal_fifo(id: &str) -> Result<PathBuf, ShimError> {
    let start_fifo_path = state_dir(id).join("start.fifo");
    nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU)?;
    Ok(start_fifo_path)
}

fn write_id_mappings(pid: Pid, config: &Config) -> Result<(), ShimError> {
    std::fs::write(
        format!("/proc/{}/uid_map", pid),
        create_id_map_contents(&config.linux.uid_mappings),
    )?;

    std::fs::write(format!("/proc/{}/setgroups", pid), "deny")?;
    std::fs::write(
        format!("/proc/{}/gid_map", pid),
        create_id_map_contents(&config.linux.gid_mappings),
    )?;
    Ok(())
}

fn create_id_map_contents(mappings: &[IdMappingConfig]) -> String {
    mappings
        .iter()
        .map(|m| format!("{} {} {}\n", m.container_id, m.host_id, m.size))
        .collect()
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

#[derive(Debug)]
pub enum ShimError {
    State(StateError),
    Config(ConfigError),
    Syscall(nix::Error),
    Io(io::Error),
    ChildReportedFailure,
    ChildExitedEarly,
    ChildSyscall,
    ChildIo,
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

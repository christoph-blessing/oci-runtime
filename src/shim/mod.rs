use std::{
    error::Error,
    fmt::Display,
    io,
    os::fd::{BorrowedFd, IntoRawFd},
    path::{Path, PathBuf},
};

use nix::{
    sys::{signal::Signal, stat::Mode, wait::WaitStatus},
    unistd::Pid,
};

use crate::{
    cmd::create::{
        ALREADY_EXISTS, CHILD_CAPS, CHILD_EXEC_NOT_FOUND, CHILD_IO, CHILD_NUL_BYTE, CHILD_SYSCALL,
        CHILD_UNEXPECTED, CONFIG, CONFIG_NOT_FOUND, CONFIG_PARSE, IO, READY, STATE, SYSCALL,
        VALIDATION,
    },
    config::{
        error::ConfigError,
        validated::{Config, IdMappingConfig},
    },
    shim::child::ChildError,
    state::{Creating, ExitReason, State, StateError, StateGuard, Stoppable},
};

pub mod child;

const STACK_SIZE: usize = 1024 * 1024;
const EXIT_SYSCALL: u8 = 2;
const EXIT_IO: u8 = 3;
const EXIT_NUL_BYTE: u8 = 4;
const EXIT_CAPS: u8 = 5;
const EXIT_EXEC_NOT_FOUND: u8 = 6;

pub fn run(id: &str, bundle: &Path, ready_fd: i32) -> Result<(), ShimError> {
    let signal_guard = ReadySignalGuard::new(ready_fd);
    let child_guard = match setup(id, bundle) {
        Ok(child_guard) => {
            nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &[READY])?;
            signal_guard.confirm();
            child_guard
        }
        Err(error) => {
            let signal = match error {
                ShimError::State(StateError::AlreadyExists(_)) => ALREADY_EXISTS,
                ShimError::State(_) => STATE,
                ShimError::ChildReported(ref e) => match e {
                    ChildReportedError::ExecutableNotFound => CHILD_EXEC_NOT_FOUND,
                    ChildReportedError::Syscall => CHILD_SYSCALL,
                    ChildReportedError::Io => CHILD_IO,
                    ChildReportedError::NulByte => CHILD_NUL_BYTE,
                    ChildReportedError::Capabilities => CHILD_CAPS,
                    ChildReportedError::UnexpectedExit(_) => CHILD_UNEXPECTED,
                },
                ShimError::Config(ConfigError::NotFound(_)) => CONFIG_NOT_FOUND,
                ShimError::Config(ConfigError::Parse(_)) => CONFIG_PARSE,
                ShimError::Config(ConfigError::Validation(_)) => VALIDATION,
                ShimError::Config(_) => CONFIG,
                ShimError::Syscall(_) => SYSCALL,
                ShimError::Io(_) => IO,
            };

            nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &[signal])?;
            signal_guard.confirm();
            return Err(error);
        }
    };

    let status = loop {
        match nix::sys::wait::waitpid(child_guard.pid, None) {
            Ok(s) => break s,
            Err(nix::Error::EINTR) => continue,
            Err(e) => return Err(e.into()),
        }
    };
    child_guard.confirm();

    match status {
        WaitStatus::Exited(_, code) => {
            stop(id, ExitReason::Exited(code))?;
        }
        WaitStatus::Signaled(_, signal, _) => {
            stop(id, ExitReason::Signaled(signal))?;
        }
        other => panic!("unexpected wait status: {:?}", other),
    };

    Ok(())
}

fn setup(id: &str, bundle: &Path) -> Result<ChildGuard, ShimError> {
    let creating = Creating::new(id, bundle.to_path_buf(), None);
    crate::state::persist(&creating.clone().into())?;
    let state_guard = StateGuard::new(id);

    let start_fifo_path = create_start_signal_fifo(id)?;
    let config = Config::new(bundle)?;

    let child_ready_pipe = SignalPipe::new()?;
    let mappings_ready_pipe = SignalPipe::new()?;

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
            Err(e) => {
                (match e {
                    ChildError::Syscall(_) => EXIT_SYSCALL,
                    ChildError::Io(_) => EXIT_IO,
                    ChildError::NulByte(_) => EXIT_NUL_BYTE,
                    ChildError::Capabilities(_) => EXIT_CAPS,
                    ChildError::ExecutableNotFound => EXIT_EXEC_NOT_FOUND,
                }) as isize
            }
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
    let child_guard = ChildGuard::new(pid);

    write_id_mappings(pid, &config)?;
    mappings_ready_pipe.send(true)?;

    match wait_for_child_setup(child_ready_pipe)? {
        ChildSetup::Success => {}
        ChildSetup::Failure => {
            let status = nix::sys::wait::waitpid(pid, None)?;
            child_guard.confirm();
            let code = match status {
                WaitStatus::Exited(_, code) => code,
                other => panic!("unexpected wait status: {:?}", other),
            };
            return Err(ShimError::ChildReported(ChildReportedError::from_code(
                code as u8,
            )));
        }
    };

    let created = creating.finish_setup(pid.as_raw(), &start_fifo_path);
    crate::state::persist(&created.into())?;
    state_guard.confirm();

    Ok(child_guard)
}

enum ChildSetup {
    Success,
    Failure,
}

fn wait_for_child_setup(pipe: SignalPipe) -> Result<ChildSetup, ShimError> {
    match pipe.recv()? {
        Some(c) => {
            if !c {
                return Ok(ChildSetup::Failure);
            }
        }
        None => return Ok(ChildSetup::Failure),
    }
    Ok(ChildSetup::Success)
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

struct ChildGuard {
    pid: Pid,
    confirmed: bool,
}

impl ChildGuard {
    fn new(pid: Pid) -> Self {
        ChildGuard {
            pid,
            confirmed: false,
        }
    }
    fn confirm(mut self) {
        self.confirmed = true;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.confirmed {
            let _ = nix::sys::signal::kill(self.pid, Signal::SIGKILL);
            let _ = nix::sys::wait::waitpid(self.pid, None);
        }
    }
}

struct SignalPipe {
    read_fd: i32,
    write_fd: i32,
}

impl SignalPipe {
    fn new() -> Result<Self, ShimError> {
        let (read, write) = nix::unistd::pipe()?;
        let read_fd = read.into_raw_fd();
        let write_fd = write.into_raw_fd();
        Ok(Self { read_fd, write_fd })
    }

    fn recv(&self) -> Result<Option<bool>, ShimError> {
        nix::unistd::close(self.write_fd)?;
        let borrowed = unsafe { BorrowedFd::borrow_raw(self.read_fd) };
        let mut buf = [0u8; 1];
        let n = nix::unistd::read(borrowed, &mut buf)?;
        nix::unistd::close(self.read_fd)?;

        let mut option = None;
        if n != 0 {
            if buf[0] == 1 {
                option = Some(true);
            } else {
                option = Some(false);
            }
        }

        Ok(option)
    }

    fn send(&self, confirm: bool) -> Result<(), ShimError> {
        let mut buf = [0u8; 1];
        if confirm {
            buf = [1u8; 1];
        }

        nix::unistd::close(self.read_fd)?;
        let borrowed = unsafe { BorrowedFd::borrow_raw(self.write_fd) };
        nix::unistd::write(borrowed, &buf)?;
        nix::unistd::close(self.write_fd)?;
        Ok(())
    }
}

fn create_start_signal_fifo(id: &str) -> Result<PathBuf, ShimError> {
    let start_fifo_path = crate::state::state_dir(id).join("start.fifo");
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

fn stop(id: &str, exit_reason: ExitReason) -> Result<(), ShimError> {
    let stopped = match crate::state::load(id)? {
        State::Created(c) => c.stop(exit_reason),
        State::Running(r) => r.stop(exit_reason),
        other => panic!("unexpected state: {}", other.as_string()),
    };
    crate::state::persist(&stopped.into())?;
    Ok(())
}

#[derive(Debug)]
pub enum ShimError {
    State(StateError),
    Config(ConfigError),
    Syscall(nix::Error),
    Io(io::Error),
    ChildReported(ChildReportedError),
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

impl Display for ShimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(_) => write!(f, "state error during shim operation"),
            Self::Config(_) => write!(f, "config error during shim operation"),
            Self::Syscall(_) => write!(f, "syscall error during shim operation"),
            Self::Io(_) => write!(f, "i/o error during shim operation"),
            Self::ChildReported(_) => write!(f, "child reported error during shim operation"),
        }
    }
}

impl Error for ShimError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(e) => Some(e),
            Self::Config(e) => Some(e),
            Self::Syscall(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::ChildReported(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum ChildReportedError {
    Syscall,
    Io,
    NulByte,
    Capabilities,
    ExecutableNotFound,
    UnexpectedExit(u8),
}

impl ChildReportedError {
    fn from_code(code: u8) -> Self {
        match code {
            EXIT_SYSCALL => Self::Syscall,
            EXIT_IO => ChildReportedError::Io,
            EXIT_NUL_BYTE => ChildReportedError::NulByte,
            EXIT_CAPS => ChildReportedError::Capabilities,
            EXIT_EXEC_NOT_FOUND => ChildReportedError::ExecutableNotFound,
            other => ChildReportedError::UnexpectedExit(other),
        }
    }
}

impl Display for ChildReportedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syscall => write!(f, "syscall error"),
            Self::Io => write!(f, "i/o error"),
            Self::NulByte => write!(f, "nul byte error"),
            Self::Capabilities => write!(f, "capabilities error"),
            Self::ExecutableNotFound => write!(f, "cannot find executable"),
            Self::UnexpectedExit(c) => write!(f, "unexpected exit code: {}", c),
        }
    }
}

impl Error for ChildReportedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Syscall
            | Self::Io
            | Self::NulByte
            | Self::Capabilities
            | Self::ExecutableNotFound
            | Self::UnexpectedExit(_) => None,
        }
    }
}

use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::path::Path;
use std::{collections::HashMap, io, path::PathBuf};

use nix::sys::signal::Signal;
use nix::sys::stat::Mode;
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};

use crate::config::error::ConfigError;
use crate::config::validated::{Config, IdMappingConfig};
use crate::shim::child::ChildError;

const STACK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum State {
    Creating(Creating),
    Created(Created),
    Running(Running),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Common {
    pub oci_version: String,
    pub id: String,
    pub bundle: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creating {
    #[serde(flatten)]
    common: Common,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    #[serde(flatten)]
    common: Common,
    pub pid: i32,
    pub internal: CreatedInternal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedInternal {
    pub start_signal: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    #[serde(flatten)]
    common: Common,
    pub pid: i32,
}

impl State {
    fn save(&self) -> Result<(), StateError> {
        let json = serde_json::to_string(self)?;
        std::fs::write(state_dir(self.id().as_str()).join("state.json"), json)?;
        Ok(())
    }

    fn load(id: &str) -> Result<Self, StateError> {
        let json = std::fs::read_to_string(state_dir(id).join("state.json")).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                StateError::NotFound(e.to_string())
            } else {
                StateError::Io(e)
            }
        })?;
        let state: State = serde_json::from_str(json.as_str())?;
        Ok(state)
    }

    fn id(&self) -> String {
        match self {
            Self::Creating(s) => s.common.id.to_string(),
            Self::Created(s) => s.common.id.to_string(),
            Self::Running(s) => s.common.id.to_string(),
        }
    }

    fn as_string(&self) -> String {
        match self {
            Self::Creating(_) => String::from("creating"),
            Self::Created(_) => String::from("created"),
            Self::Running(_) => String::from("running"),
        }
    }
}

impl Creating {
    pub fn new(
        id: &str,
        bundle: PathBuf,
        annotations: Option<HashMap<String, String>>,
    ) -> Result<Self, StateError> {
        let state = Self {
            common: Common {
                oci_version: String::from("1.3.0"),
                id: id.to_string(),
                bundle,
                annotations,
            },
        };
        if state_dir(id).exists() {
            return Err(StateError::AlreadyExists(id.to_string()));
        }
        std::fs::create_dir_all(state_dir(id))?;
        state.save()?;
        Ok(state)
    }

    pub fn load(id: &str) -> Result<Self, StateError> {
        match State::load(id)? {
            State::Creating(c) => Ok(c),
            other => Err(StateError::InvalidState {
                state: other.as_string(),
            }),
        }
    }

    fn save(&self) -> Result<(), StateError> {
        let state = State::Creating(self.clone());
        state.save()
    }

    pub fn state_dir(&self) -> PathBuf {
        state_dir(self.common.id.as_str())
    }

    pub fn finish_setup(self, done_fd: i32) -> Result<Created, StateError> {
        let created = match self.setup_child() {
            Ok(p) => {
                Self::send_done_signal(done_fd, true)?;
                p
            }
            Err(e) => {
                Self::send_done_signal(done_fd, false)?;
                return Err(e);
            }
        };
        Ok(created)
    }

    fn setup_child(self) -> Result<Created, StateError> {
        let guard = StateGuard::new(&self.common.id);
        let start_fifo_path = self.create_start_signal_fifo()?;
        let config = Config::new(self.common.bundle.as_path())?;
        let pid = Self::clone_child(&config, &start_fifo_path)?;

        let created = Created {
            common: Common {
                oci_version: self.common.oci_version,
                id: self.common.id,
                bundle: self.common.bundle,
                annotations: self.common.annotations,
            },
            pid: pid.as_raw(),
            internal: CreatedInternal {
                start_signal: start_fifo_path.to_path_buf(),
            },
        };
        created.save()?;
        guard.confirm();
        Ok(created)
    }

    fn create_start_signal_fifo(&self) -> Result<PathBuf, StateError> {
        let start_fifo_path = self.state_dir().join("start.fifo");
        nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU)?;
        Ok(start_fifo_path)
    }

    fn clone_child(config: &Config, start_fifo_path: &Path) -> Result<Pid, StateError> {
        let mut stack = vec![0u8; STACK_SIZE];

        let (read_mappings_ready, write_mappings_ready) = nix::unistd::pipe()?;
        let read_mappings_ready_fd = read_mappings_ready.into_raw_fd();
        let write_mappings_ready_fd = write_mappings_ready.into_raw_fd();

        let cb = Box::new(|| {
            match crate::shim::child::run(
                &config,
                read_mappings_ready_fd,
                write_mappings_ready_fd,
                start_fifo_path,
            ) {
                Ok(_) => 0,
                Err(e) => match e {
                    ChildError::Syscall(_) => 2,
                    ChildError::Io(_) => 3,
                },
            }
        });
        let pid = unsafe {
            nix::sched::clone(
                cb,
                &mut stack,
                config.linux.clone_flags,
                Some(Signal::SIGCHLD as i32),
            )
        }?;

        nix::unistd::close(read_mappings_ready_fd)?;
        Self::write_id_mappings(pid, config)?;
        Self::send_mappings_ready_signal(write_mappings_ready_fd)?;

        Ok(pid)
    }

    fn write_id_mappings(pid: Pid, config: &Config) -> Result<(), StateError> {
        std::fs::write(
            format!("/proc/{}/uid_map", pid),
            Self::create_id_map_contents(&config.linux.uid_mappings),
        )?;

        std::fs::write(format!("/proc/{}/setgroups", pid), "deny")?;
        std::fs::write(
            format!("/proc/{}/gid_map", pid),
            Self::create_id_map_contents(&config.linux.gid_mappings),
        )?;
        Ok(())
    }

    fn send_mappings_ready_signal(write_fd: i32) -> Result<(), StateError> {
        let mut buffer = vec![0u8; 1];
        let borrowed = unsafe { BorrowedFd::borrow_raw(write_fd) };
        nix::unistd::write(borrowed, &mut buffer)?;
        nix::unistd::close(write_fd)?;
        Ok(())
    }

    fn create_id_map_contents(mappings: &[IdMappingConfig]) -> String {
        mappings
            .iter()
            .map(|m| format!("{} {} {}\n", m.container_id, m.host_id, m.size))
            .collect()
    }

    fn send_done_signal(done_fd: i32, is_success: bool) -> Result<(), StateError> {
        let mut buf;
        if is_success {
            buf = [1u8; 1];
        } else {
            buf = [0u8; 1];
        }
        nix::unistd::write(unsafe { BorrowedFd::borrow_raw(done_fd) }, &mut buf)?;
        Ok(())
    }
}

impl Created {
    pub fn load(id: &str) -> Result<Self, StateError> {
        match State::load(id)? {
            State::Created(c) => Ok(c),
            other => Err(StateError::InvalidState {
                state: other.as_string(),
            }),
        }
    }

    fn save(&self) -> Result<(), StateError> {
        let state = State::Created(self.clone());
        state.save()
    }

    pub fn start(self) -> Result<Running, StateError> {
        self.send_start_signal()?;

        let state = Running {
            common: Common {
                oci_version: self.common.oci_version,
                id: self.common.id,
                bundle: self.common.bundle,
                annotations: self.common.annotations,
            },
            pid: self.pid,
        };
        state.save()?;
        Ok(state)
    }

    fn send_start_signal(&self) -> Result<(), StateError> {
        let mut start_fifo = OpenOptions::new()
            .write(true)
            .open(&self.internal.start_signal)?;
        let mut buffer = [0u8; 1];
        start_fifo.write(&mut buffer)?;
        std::fs::remove_file(&self.internal.start_signal)?;
        Ok(())
    }
}

impl Running {
    fn save(&self) -> Result<(), StateError> {
        let state = State::Running(self.clone());
        state.save()
    }
}

#[derive(Debug)]
pub enum StateError {
    NotFound(String),
    AlreadyExists(String),
    Json(serde_json::Error),
    Io(io::Error),
    Syscall(nix::Error),
    Config(ConfigError),
    InvalidState { state: String },
}

impl From<serde_json::Error> for StateError {
    fn from(value: serde_json::Error) -> Self {
        StateError::Json(value)
    }
}

impl From<io::Error> for StateError {
    fn from(value: io::Error) -> Self {
        StateError::Io(value)
    }
}

impl From<nix::Error> for StateError {
    fn from(value: nix::Error) -> Self {
        StateError::Syscall(value)
    }
}

impl From<ConfigError> for StateError {
    fn from(value: ConfigError) -> Self {
        StateError::Config(value)
    }
}

pub fn state_dir(id: &str) -> PathBuf {
    PathBuf::from(format!(
        "/run/user/{}/oci-runtime/{}",
        nix::unistd::getuid(),
        id
    ))
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

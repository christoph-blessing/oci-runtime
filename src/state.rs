use std::fmt::Display;
use std::path::Path;
use std::{collections::HashMap, io, path::PathBuf};

use nix::sys::signal::Signal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum State {
    Creating(Creating),
    Created(Created),
    Running(Running),
    Stopped(Stopped),
}

impl State {
    fn id(&self) -> String {
        match self {
            Self::Creating(s) => s.common.id.to_string(),
            Self::Created(s) => s.common.id.to_string(),
            Self::Running(s) => s.common.id.to_string(),
            Self::Stopped(s) => s.common.id.to_string(),
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Creating(_) => String::from("creating"),
            Self::Created(_) => String::from("created"),
            Self::Running(_) => String::from("running"),
            Self::Stopped(_) => String::from("stopped"),
        }
    }
}

impl From<Creating> for State {
    fn from(value: Creating) -> Self {
        Self::Creating(value)
    }
}

impl From<Created> for State {
    fn from(value: Created) -> Self {
        Self::Created(value)
    }
}

impl From<Running> for State {
    fn from(value: Running) -> Self {
        Self::Running(value)
    }
}

impl From<Stopped> for State {
    fn from(value: Stopped) -> Self {
        Self::Stopped(value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creating {
    #[serde(flatten)]
    common: Common,
}

impl Creating {
    pub fn new(id: &str, bundle: PathBuf, annotations: Option<HashMap<String, String>>) -> Self {
        Self {
            common: Common {
                oci_version: String::from("1.3.0"),
                id: id.to_string(),
                bundle,
                annotations,
            },
        }
    }

    pub fn finish_setup(self, pid: i32, start_fifo_path: &Path) -> Created {
        Created {
            common: Common {
                oci_version: self.common.oci_version,
                id: self.common.id,
                bundle: self.common.bundle,
                annotations: self.common.annotations,
            },
            pid: pid,
            internal: CreatedInternal {
                start_signal: start_fifo_path.to_path_buf(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    #[serde(flatten)]
    common: Common,
    pub pid: i32,
    pub internal: CreatedInternal,
}

impl Created {
    pub fn start(self) -> Running {
        Running {
            common: Common {
                oci_version: self.common.oci_version,
                id: self.common.id,
                bundle: self.common.bundle,
                annotations: self.common.annotations,
            },
            pid: self.pid,
        }
    }
}

impl Stoppable for Created {
    fn into_common(self) -> Common {
        self.common
    }
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

impl Stoppable for Running {
    fn into_common(self) -> Common {
        self.common
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    #[serde(flatten)]
    common: Common,
    internal: StoppedInternal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoppedInternal {
    exit_reason: ExitReason,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExitReason {
    Exited(i32),
    Signaled(#[serde(with = "signal_serde")] Signal),
}

mod signal_serde {
    use nix::sys::signal::Signal;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(signal: &Signal, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i32(*signal as i32)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Signal, D::Error> {
        let num = i32::deserialize(d)?;
        Signal::try_from(num).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Common {
    pub oci_version: String,
    pub id: String,
    pub bundle: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug)]
pub enum StateError {
    NotFound(String),
    AlreadyExists(String),
    Json(serde_json::Error),
    Io(io::Error),
    Syscall(nix::Error),
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

impl Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "container not found: {}", id),
            Self::AlreadyExists(id) => write!(f, "container already exists: {}", id),
            Self::Json(e) => write!(f, "JSON error: {}", e),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Syscall(e) => write!(f, "syscall error: {}", e),
        }
    }
}

pub fn state_dir(id: &str) -> PathBuf {
    PathBuf::from(format!(
        "/run/user/{}/oci-runtime/{}",
        nix::unistd::getuid(),
        id
    ))
}

pub fn persist(state: &State) -> Result<(), StateError> {
    if let State::Creating(_) = state {
        if state_dir(&state.id()).exists() {
            return Err(StateError::AlreadyExists(state.id().to_string()));
        }
        std::fs::create_dir_all(state_dir(&state.id()))?;
    }
    let json = serde_json::to_string(state)?;
    std::fs::write(state_dir(state.id().as_str()).join("state.json"), json)?;
    Ok(())
}

pub fn load(id: &str) -> Result<State, StateError> {
    let json = std::fs::read_to_string(state_dir(id).join("state.json")).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            StateError::NotFound(id.to_string())
        } else {
            StateError::Io(e)
        }
    })?;
    let state: State = serde_json::from_str(json.as_str())?;
    Ok(state)
}

pub fn exists(id: &str) -> bool {
    state_dir(id).join("state.json").exists()
}

pub struct StateGuard {
    dir: PathBuf,
    confirmed: bool,
}

impl StateGuard {
    pub fn new(id: &str) -> Self {
        Self {
            dir: crate::state::state_dir(id),
            confirmed: false,
        }
    }

    pub fn confirm(mut self) {
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

pub trait Stoppable {
    fn into_common(self) -> Common;

    fn stop(self, exit_reason: ExitReason) -> Stopped
    where
        Self: Sized,
    {
        Stopped {
            common: self.into_common(),
            internal: StoppedInternal { exit_reason },
        }
    }
}

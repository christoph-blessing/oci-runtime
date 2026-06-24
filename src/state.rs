use std::fs;
use std::path::Path;
use std::{collections::HashMap, io, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum State {
    Creating(Creating),
    Created(Created),
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    #[serde(flatten)]
    common: Common,
    pub pid: i32,
    pub internal: Internal,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Internal {
    pub start_signal: PathBuf,
}

impl State {
    pub fn new(
        id: &str,
        bundle: PathBuf,
        annotations: Option<HashMap<String, String>>,
    ) -> Result<Self, StateError> {
        let state = Self::Creating(Creating {
            common: Common {
                oci_version: String::from("1.3.0"),
                id: id.to_string(),
                bundle,
                annotations,
            },
        });
        fs::create_dir(state_dir(id)?).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                StateError::AlreadyExists(id.to_string())
            } else {
                StateError::Io(e)
            }
        })?;
        state.save()?;
        Ok(state)
    }

    pub fn finish_setup(&self, pid: i32, start_fifo: &Path) -> Result<Self, StateError> {
        match self {
            Self::Creating(c) => {
                let new_state = Self::Created(c.clone().finish_setup(pid, start_fifo));
                new_state.save()?;
                Ok(new_state)
            }
            other => {
                return Err(format!("cannot finish container setup in state: {:?}", other).into());
            }
        }
    }

    fn save(&self) -> Result<(), StateError> {
        let json = serde_json::to_string(self)?;
        fs::write(state_dir(self.id().as_str())?.join("state.json"), json)?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self, StateError> {
        let json = fs::read_to_string(state_dir(id)?.join("state.json")).map_err(|e| {
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
        }
    }

    pub fn state_dir(&self) -> Result<PathBuf, StateError> {
        Ok(state_dir(&self.id())?)
    }
}

impl Creating {
    fn finish_setup(self, pid: i32, start_fifo: &Path) -> Created {
        Created {
            common: Common {
                oci_version: self.common.oci_version,
                id: self.common.id,
                bundle: self.common.bundle,
                annotations: self.common.annotations,
            },
            pid: pid,
            internal: Internal {
                start_signal: start_fifo.to_path_buf(),
            },
        }
    }
}

#[derive(Debug)]
pub enum StateError {
    NotFound(String),
    AlreadyExists(String),
    Json(serde_json::Error),
    Io(io::Error),
    Transition(String),
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

impl From<String> for StateError {
    fn from(value: String) -> Self {
        Self::Transition(value)
    }
}

fn state_dir(id: &str) -> Result<PathBuf, StateError> {
    let runtime_dir = PathBuf::from(format!("/run/user/{}/oci-runtime", nix::unistd::getuid()));
    fs::create_dir_all(runtime_dir)?;
    Ok(PathBuf::from(format!(
        "/run/user/{}/oci-runtime/{}",
        nix::unistd::getuid(),
        id
    )))
}

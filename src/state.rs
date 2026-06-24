use std::fs;
use std::{collections::HashMap, io, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    pub oci_version: String,
    pub id: String,
    pub status: Status,
    pub pid: Option<i32>,
    pub bundle: PathBuf,
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Status {
    Creating,
    Created,
    Running,
    Stopped,
}

impl State {
    pub fn new(
        id: String,
        status: Status,
        pid: Option<i32>,
        bundle: PathBuf,
        annotations: Option<HashMap<String, String>>,
    ) -> Self {
        Self {
            oci_version: String::from("1.3.0"),
            id,
            status,
            pid,
            bundle,
            annotations,
        }
    }

    pub fn save(&self) -> Result<(), StateError> {
        let json = serde_json::to_string(self)?;
        fs::write(state_dir(self.id.as_str()).join("state.json"), json)?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum StateError {
    Json(serde_json::Error),
    Io(io::Error),
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

fn state_dir(id: &str) -> PathBuf {
    PathBuf::from(format!(
        "/run/user/{}/oci-runtime/{}",
        nix::unistd::getuid(),
        id
    ))
}

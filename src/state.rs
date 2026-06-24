use std::{collections::HashMap, path::PathBuf};

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
}

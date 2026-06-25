use super::raw::NamespaceKind;
use std::{fmt::Display, io, path::PathBuf};

#[derive(Debug)]
pub enum ConfigError {
    NotFound(PathBuf),
    Validation(ValidationErrors),
    Io(io::Error),
    Parse(serde_json::Error),
}

impl From<serde_json::Error> for ConfigError {
    fn from(value: serde_json::Error) -> Self {
        ConfigError::Parse(value)
    }
}

impl From<ValidationErrors> for ConfigError {
    fn from(value: ValidationErrors) -> Self {
        ConfigError::Validation(value)
    }
}

#[derive(Debug)]
pub struct ValidationErrors(pub Vec<ValidationError>);

#[derive(Debug)]
pub enum ValidationError {
    InvalidVersion(String),
    UnsupportedVersion,
    PathNotFound(PathBuf),
    NotADirectory(PathBuf),
    EmptyArgs,
    DuplicateNamespace(NamespaceKind),
}

impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVersion(e) => write!(f, "ociVersion is invalid: {}", e),
            Self::UnsupportedVersion => write!(f, "ociVersion is unsupported"),
            Self::PathNotFound(p) => write!(f, "root.path does not exist: {}", p.display()),
            Self::NotADirectory(p) => write!(f, "root.path is not a directory: {}", p.display()),
            Self::EmptyArgs => write!(f, "process.args must contain at least one argument"),
            Self::DuplicateNamespace(n) => {
                write!(f, "linux.namespaces contains duplicates: {}", n)
            }
        }
    }
}

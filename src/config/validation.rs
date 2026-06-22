use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;

use semver::Version;
use semver::VersionReq;

use super::raw::Config as RawConfig;
use super::raw::NamespaceKind;
use super::validated::Config as ValidatedConfig;
use super::validated::LinuxConfig as ValidatedLinuxConfig;
use super::validated::MountConfig as ValidatedMountConfig;
use super::validated::ProcessConfig as ValidatedProcessConfig;
use super::validated::RootConfig as ValidatedRootConfig;

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

pub fn validate(raw_config: RawConfig) -> Result<ValidatedConfig, Vec<ValidationError>> {
    let mut errors = Vec::new();

    let version = match validate_version(raw_config.oci_version) {
        Ok(version) => Some(version),
        Err(error) => {
            errors.push(error);
            None
        }
    };

    let root_path = match validate_root_path(raw_config.root.path) {
        Ok(root_path) => Some(root_path),
        Err(error) => {
            errors.push(error);
            None
        }
    };

    let mut mounts = Vec::new();
    if let Some(raw_mounts) = raw_config.mounts {
        for raw_mount in raw_mounts {
            mounts.push(ValidatedMountConfig::from(raw_mount));
        }
    }

    let mut process = None;
    if let Some(raw_process) = raw_config.process {
        process = match ValidatedProcessConfig::try_from(raw_process) {
            Ok(process) => Some(process),
            Err(error) => {
                errors.push(error);
                None
            }
        };
    }

    let linux = match ValidatedLinuxConfig::try_from(raw_config.linux) {
        Ok(linux) => Some(linux),
        Err(error) => {
            errors.push(error);
            None
        }
    };

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(ValidatedConfig {
        oci_version: version.unwrap(),
        hostname: raw_config.hostname,
        root: ValidatedRootConfig {
            path: root_path.unwrap(),
            readonly: raw_config.root.readonly.unwrap_or(false),
        },
        mounts,
        process,
        linux: linux.unwrap(),
    })
}

fn validate_version(version: String) -> Result<Version, ValidationError> {
    let mut version_result = Version::parse(version.as_str())
        .map_err(|e| ValidationError::InvalidVersion(e.to_string()));
    if let Ok(ref config_version) = version_result {
        let req = VersionReq::parse("^1.0").unwrap();
        if !req.matches(config_version) {
            version_result = Err(ValidationError::UnsupportedVersion)
        }
    }
    version_result
}

#[derive(Debug)]
pub struct ExistingDir(PathBuf);

impl ExistingDir {
    fn new(path: PathBuf) -> Result<Self, ValidationError> {
        if !path.exists() {
            return Err(ValidationError::PathNotFound(path));
        }
        if !path.is_dir() {
            return Err(ValidationError::NotADirectory(path));
        }
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

fn validate_root_path(path: PathBuf) -> Result<ExistingDir, ValidationError> {
    ExistingDir::new(path)
}

// Mount validation
#[derive(Debug)]
pub struct AbsolutePath(PathBuf);

impl AbsolutePath {
    pub fn new(path: PathBuf) -> Self {
        Self(PathBuf::from("/").join(path))
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

#[cfg(test)]
mod tests {
    use crate::config::raw::LinuxConfig;
    use tempfile::NamedTempFile;

    use crate::config::raw::MountConfig;
    use crate::config::raw::NamespaceConfig;
    use crate::config::raw::ProcessConfig;
    use crate::config::raw::UserConfig;

    use super::*;

    #[test]
    fn test_invalid_version() {
        let err = validate_version(String::from("1.2.3.4")).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidVersion(_)))
    }

    #[test]
    fn test_unsupported_version() {
        let err = validate_version(String::from("2.3.3")).unwrap_err();
        assert!(matches!(err, ValidationError::UnsupportedVersion))
    }

    #[test]
    fn test_missing_root_path() {
        let err = validate_root_path(PathBuf::from("/does/not/exist")).unwrap_err();
        assert!(matches!(err, ValidationError::PathNotFound(_)))
    }

    #[test]
    fn test_root_path_not_a_directory() {
        let file = NamedTempFile::new().unwrap();
        let err = validate_root_path(file.path().to_owned()).unwrap_err();
        assert!(matches!(err, ValidationError::NotADirectory(_)));
    }

    #[test]
    fn test_mount_destination_absolute() {
        let config = MountConfig {
            destination: PathBuf::from("not/absolute"),
            kind: None,
            source: None,
            options: None,
        };
        assert!(
            ValidatedMountConfig::from(config)
                .destination
                .as_path()
                .is_absolute()
        )
    }

    #[test]
    fn test_empty_process_args() {
        let config = ProcessConfig {
            cwd: PathBuf::from("/some/path"),
            env: None,
            args: Vec::new(),
            user: UserConfig { uid: 0, gid: 0 },
            capabilities: None,
            no_new_privileges: None,
            rlimits: None,
        };
        let err = ValidatedProcessConfig::try_from(config).unwrap_err();
        assert!(matches!(err, ValidationError::EmptyArgs));
    }

    #[test]
    fn test_no_new_privileges_none() {
        let config = ProcessConfig {
            cwd: PathBuf::from("/some/path"),
            env: None,
            args: vec![String::from("ls")],
            user: UserConfig { uid: 0, gid: 0 },
            capabilities: None,
            no_new_privileges: None,
            rlimits: None,
        };
        assert!(
            ValidatedProcessConfig::try_from(config)
                .unwrap()
                .no_new_privileges
                == false
        );
    }

    #[test]
    fn test_duplicate_namespaces() {
        let config = LinuxConfig {
            namespaces: vec![
                NamespaceConfig {
                    kind: NamespaceKind::Pid,
                },
                NamespaceConfig {
                    kind: NamespaceKind::Pid,
                },
            ],
            gid_mappings: None,
            uid_mappings: None,
            masked_paths: None,
            readonly_paths: None,
        };
        let err = ValidatedLinuxConfig::try_from(config).unwrap_err();
        assert!(matches!(err, ValidationError::DuplicateNamespace(_)));
    }
}

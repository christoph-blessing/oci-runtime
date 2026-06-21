use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;

use nix::mount::MsFlags;
use semver::Version;
use semver::VersionReq;

use crate::config::raw::MountConfig;
use crate::config::raw::ProcessConfig;

use super::raw::Config as RawConfig;
use super::validated::CapabilitiesConfig as ValidatedCapabilitiesConfig;
use super::validated::Config as ValidatedConfig;
use super::validated::MountConfig as ValidatedMountConfig;
use super::validated::ProcessConfig as ValidatedProcessConfig;
use super::validated::RootConfig as ValidatedRootConfig;
use super::validated::UserConfig as ValidatedUserConfig;

pub enum ValidationError {
    InvalidVersion(String),
    UnsupportedVersion,
    PathNotFound(PathBuf),
    NotADirectory(PathBuf),
    EmptyArgs,
}

impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVersion(e) => write!(f, "ociVersion is invalid: {}", e),
            Self::UnsupportedVersion => write!(f, "ociVersion is unsupported"),
            Self::PathNotFound(p) => write!(f, "root.path does not exist: {}", p.display()),
            Self::NotADirectory(p) => write!(f, "root.path is not a directory: {}", p.display()),
            Self::EmptyArgs => write!(f, "process.args must contain at least one argument"),
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
            mounts.push(validate_mount(raw_mount));
        }
    }

    let mut process = None;
    if let Some(raw_process) = raw_config.process {
        process = match validate_process(raw_process) {
            Ok(process) => Some(process),
            Err(error) => {
                errors.push(error);
                None
            }
        };
    }

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
    fn new(path: PathBuf) -> Self {
        Self(PathBuf::from("/").join(path))
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

fn validate_mount(config: MountConfig) -> ValidatedMountConfig {
    let mut flags = MsFlags::empty();
    for option in config.options.unwrap_or_default() {
        match option.as_str() {
            "async" => flags &= !MsFlags::MS_SYNCHRONOUS,
            "atime" => flags &= !MsFlags::MS_NOATIME,
            "bind" => flags |= MsFlags::MS_BIND,
            "defaults" => {}
            "dev" => flags &= !MsFlags::MS_NODEV,
            "diratime" => flags &= !MsFlags::MS_NODIRATIME,
            "dirsync" => flags |= MsFlags::MS_DIRSYNC,
            "exec" => flags &= !MsFlags::MS_NOEXEC,
            "iversion" => flags |= MsFlags::MS_I_VERSION,
            "lazytime" => flags |= MsFlags::MS_LAZYTIME,
            "loud" => flags &= !MsFlags::MS_SILENT,
            "mand" => flags |= MsFlags::MS_MANDLOCK,
            "noatime" => flags |= MsFlags::MS_NOATIME,
            "nodev" => flags |= MsFlags::MS_NODEV,
            "nodiratime" => flags |= MsFlags::MS_NODIRATIME,
            "noexec" => flags |= MsFlags::MS_NOEXEC,
            "noiversion" => flags &= !MsFlags::MS_I_VERSION,
            "nolazytime" => flags &= !MsFlags::MS_LAZYTIME,
            "nomand" => flags &= !MsFlags::MS_MANDLOCK,
            "norelatime" => flags &= !MsFlags::MS_RELATIME,
            "nostrictatime" => flags &= !MsFlags::MS_STRICTATIME,
            "nosuid" => flags |= MsFlags::MS_NOSUID,
            "private" => flags |= MsFlags::MS_PRIVATE,
            "rbind" => flags |= MsFlags::MS_BIND | MsFlags::MS_REC,
            "relatime" => flags |= MsFlags::MS_RELATIME,
            "remount" => flags |= MsFlags::MS_REMOUNT,
            "ro" => flags |= MsFlags::MS_RDONLY,
            "rprivate" => flags |= MsFlags::MS_PRIVATE | MsFlags::MS_REC,
            "rshared" => flags |= MsFlags::MS_SHARED | MsFlags::MS_REC,
            "rslave" => flags |= MsFlags::MS_SLAVE | MsFlags::MS_REC,
            "runbindable" => flags |= MsFlags::MS_UNBINDABLE | MsFlags::MS_REC,
            "rw" => flags &= !MsFlags::MS_RDONLY,
            "shared" => flags |= MsFlags::MS_SHARED,
            "silent" => flags |= MsFlags::MS_SILENT,
            "slave" => flags |= MsFlags::MS_SLAVE,
            "strictatime" => flags |= MsFlags::MS_STRICTATIME,
            "suid" => flags &= !MsFlags::MS_NOSUID,
            "sync" => flags |= MsFlags::MS_SYNCHRONOUS,
            "unbindable" => flags |= MsFlags::MS_UNBINDABLE,
            _ => {}
        }
    }
    ValidatedMountConfig {
        destination: AbsolutePath::new(config.destination),
        kind: config.kind,
        source: config.source,
        flags,
    }
}

fn validate_process(config: ProcessConfig) -> Result<ValidatedProcessConfig, ValidationError> {
    if config.args.is_empty() {
        return Err(ValidationError::EmptyArgs);
    }
    let user = ValidatedUserConfig {
        uid: config.user.uid,
        gid: config.user.gid,
    };
    let mut capabilities = None;
    if let Some(raw_capabilites) = config.capabilities {
        capabilities = Some(ValidatedCapabilitiesConfig {
            effective: raw_capabilites.effective,
            bounding: raw_capabilites.bounding,
            inheritable: raw_capabilites.inheritable,
            permitted: raw_capabilites.permitted,
            ambient: raw_capabilites.ambient,
        })
    }

    Ok(ValidatedProcessConfig {
        cwd: AbsolutePath::new(config.cwd),
        env: config.env.unwrap_or_default(),
        args: config.args,
        user,
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

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
        assert!(validate_mount(config).destination.as_path().is_absolute())
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
        let err = validate_process(config).unwrap_err();
        assert!(matches!(err, ValidationError::EmptyArgs));
    }
}

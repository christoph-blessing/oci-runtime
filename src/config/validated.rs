use std::path::Path;
use std::path::PathBuf;

use crate::config::validation::ValidationError;

use super::raw;
use caps::CapsHashSet;
use nix::mount::MsFlags;
use nix::sched::CloneFlags;
use nix::sys::resource::Resource;
use semver::Version;

#[derive(Debug)]
pub struct Config {
    pub oci_version: Version,
    pub hostname: Option<String>,
    pub root: RootConfig,
    pub mounts: Vec<MountConfig>,
    pub process: Option<ProcessConfig>,
    pub linux: LinuxConfig,
}

#[derive(Debug)]
pub struct RootConfig {
    pub path: ExistingDir,
    pub readonly: bool,
}

#[derive(Debug)]
pub struct MountConfig {
    pub destination: AbsolutePath,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub flags: MsFlags,
}

#[derive(Debug)]
pub struct ProcessConfig {
    pub cwd: AbsolutePath,
    pub env: Vec<String>,
    pub args: Vec<String>,
    pub user: UserConfig,
    pub capabilities: CapabilitiesConfig,
    pub no_new_privileges: bool,
    pub rlimits: Vec<RlimitConfig>,
}

#[derive(Debug)]
pub struct UserConfig {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug)]
pub struct CapabilitiesConfig {
    pub effective: CapsHashSet,
    pub bounding: CapsHashSet,
    pub inheritable: CapsHashSet,
    pub permitted: CapsHashSet,
    pub ambient: CapsHashSet,
}

#[derive(Debug)]
pub struct RlimitConfig {
    pub resource: Resource,
    pub soft: u64,
    pub hard: u64,
}

#[derive(Debug)]
pub struct LinuxConfig {
    pub clone_flags: CloneFlags,
    pub uid_mappings: Vec<IdMappingConfig>,
    pub gid_mappings: Vec<IdMappingConfig>,
    pub masked_paths: Vec<AbsolutePath>,
    pub readonly_paths: Vec<AbsolutePath>,
}

#[derive(Debug)]
pub struct IdMappingConfig {
    pub container_id: usize,
    pub host_id: usize,
    pub size: usize,
}

impl From<raw::IdMappingConfig> for IdMappingConfig {
    fn from(value: raw::IdMappingConfig) -> Self {
        IdMappingConfig {
            container_id: value.container_id,
            host_id: value.host_id,
            size: value.size,
        }
    }
}

impl TryFrom<raw::RootConfig> for RootConfig {
    type Error = ValidationError;
    fn try_from(value: raw::RootConfig) -> Result<Self, Self::Error> {
        let path = match ExistingDir::new(value.path) {
            Ok(path) => path,
            Err(error) => return Err(error),
        };

        Ok(Self {
            path,
            readonly: value.readonly.unwrap_or(false),
        })
    }
}

impl From<raw::MountConfig> for MountConfig {
    fn from(value: raw::MountConfig) -> Self {
        let mut flags = MsFlags::empty();
        for option in value.options.unwrap_or_default() {
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
        Self {
            destination: AbsolutePath::new(value.destination),
            kind: value.kind,
            source: value.source,
            flags,
        }
    }
}

impl TryFrom<raw::ProcessConfig> for ProcessConfig {
    type Error = ValidationError;
    fn try_from(value: raw::ProcessConfig) -> Result<Self, Self::Error> {
        if value.args.is_empty() {
            return Err(ValidationError::EmptyArgs);
        }
        let user = UserConfig::from(value.user);
        let capabilities;
        if let Some(raw_capabilites) = value.capabilities {
            capabilities = CapabilitiesConfig::from(raw_capabilites)
        } else {
            capabilities = CapabilitiesConfig {
                effective: CapsHashSet::new(),
                bounding: CapsHashSet::new(),
                inheritable: CapsHashSet::new(),
                permitted: CapsHashSet::new(),
                ambient: CapsHashSet::new(),
            }
        }
        let no_new_privileges = value.no_new_privileges.unwrap_or(false);
        let rlimits = value
            .rlimits
            .unwrap_or_default()
            .into_iter()
            .map(|l| RlimitConfig::from(l))
            .collect();

        Ok(Self {
            cwd: AbsolutePath::new(value.cwd),
            env: value.env.unwrap_or_default(),
            args: value.args,
            user,
            capabilities,
            no_new_privileges,
            rlimits,
        })
    }
}

impl From<raw::UserConfig> for UserConfig {
    fn from(value: raw::UserConfig) -> Self {
        Self {
            uid: value.uid,
            gid: value.gid,
        }
    }
}

impl From<raw::CapabilitiesConfig> for CapabilitiesConfig {
    fn from(value: raw::CapabilitiesConfig) -> Self {
        Self {
            effective: value.effective.unwrap_or_default(),
            bounding: value.bounding.unwrap_or_default(),
            inheritable: value.inheritable.unwrap_or_default(),
            permitted: value.permitted.unwrap_or_default(),
            ambient: value.ambient.unwrap_or_default(),
        }
    }
}

impl From<raw::RlimitConfig> for RlimitConfig {
    fn from(value: raw::RlimitConfig) -> Self {
        let resource = match value.kind {
            raw::RlimitKind::As => Resource::RLIMIT_AS,
            raw::RlimitKind::Core => Resource::RLIMIT_CORE,
            raw::RlimitKind::Cpu => Resource::RLIMIT_CPU,
            raw::RlimitKind::Data => Resource::RLIMIT_DATA,
            raw::RlimitKind::Fsize => Resource::RLIMIT_FSIZE,
            raw::RlimitKind::Locks => Resource::RLIMIT_LOCKS,
            raw::RlimitKind::Memlock => Resource::RLIMIT_MEMLOCK,
            raw::RlimitKind::Msgqueue => Resource::RLIMIT_MSGQUEUE,
            raw::RlimitKind::Nice => Resource::RLIMIT_NICE,
            raw::RlimitKind::Nofile => Resource::RLIMIT_NOFILE,
            raw::RlimitKind::Nproc => Resource::RLIMIT_NPROC,
            raw::RlimitKind::Rss => Resource::RLIMIT_RSS,
            raw::RlimitKind::Rtprio => Resource::RLIMIT_RTPRIO,
            raw::RlimitKind::Rttime => Resource::RLIMIT_RTTIME,
            raw::RlimitKind::Sigpending => Resource::RLIMIT_SIGPENDING,
            raw::RlimitKind::Stack => Resource::RLIMIT_STACK,
        };
        Self {
            resource,
            soft: value.soft,
            hard: value.hard,
        }
    }
}

impl TryFrom<raw::LinuxConfig> for LinuxConfig {
    type Error = ValidationError;

    fn try_from(value: raw::LinuxConfig) -> Result<Self, Self::Error> {
        let mut clone_flags = CloneFlags::empty();
        for raw_namespace in value.namespaces {
            let clone_flag = match raw_namespace.kind {
                raw::NamespaceKind::Pid => CloneFlags::CLONE_NEWPID,
                raw::NamespaceKind::Network => CloneFlags::CLONE_NEWNET,
                raw::NamespaceKind::Ipc => CloneFlags::CLONE_NEWIPC,
                raw::NamespaceKind::Uts => CloneFlags::CLONE_NEWUTS,
                raw::NamespaceKind::Mount => CloneFlags::CLONE_NEWNS,
                raw::NamespaceKind::Cgroup => CloneFlags::CLONE_NEWCGROUP,
                raw::NamespaceKind::User => CloneFlags::CLONE_NEWUSER,
            };
            if clone_flags.contains(clone_flag) {
                return Err(ValidationError::DuplicateNamespace(raw_namespace.kind));
            }
            clone_flags |= clone_flag;
        }

        let uid_mappings = value
            .uid_mappings
            .unwrap_or_default()
            .into_iter()
            .map(|m| IdMappingConfig::from(m))
            .collect();
        let gid_mappings = value
            .gid_mappings
            .unwrap_or_default()
            .into_iter()
            .map(|m| IdMappingConfig::from(m))
            .collect();

        let masked_paths = value
            .masked_paths
            .unwrap_or_default()
            .into_iter()
            .map(|p| AbsolutePath::new(p))
            .collect();

        let readonly_paths = value
            .readonly_paths
            .unwrap_or_default()
            .into_iter()
            .map(|p| AbsolutePath::new(p))
            .collect();

        Ok(Self {
            clone_flags,
            uid_mappings,
            gid_mappings,
            masked_paths,
            readonly_paths,
        })
    }
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

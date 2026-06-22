use crate::config::validation::ValidationError;

use super::raw;
use super::validation::AbsolutePath;
use super::validation::ExistingDir;
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

impl From<raw::UserConfig> for UserConfig {
    fn from(value: raw::UserConfig) -> Self {
        Self {
            uid: value.uid,
            gid: value.gid,
        }
    }
}

#[derive(Debug)]
pub struct CapabilitiesConfig {
    pub effective: CapsHashSet,
    pub bounding: CapsHashSet,
    pub inheritable: CapsHashSet,
    pub permitted: CapsHashSet,
    pub ambient: CapsHashSet,
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

#[derive(Debug)]
pub struct RlimitConfig {
    pub resource: Resource,
    pub soft: u64,
    pub hard: u64,
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

#[derive(Debug)]
pub struct LinuxConfig {
    pub clone_flags: CloneFlags,
    pub uid_mappings: Vec<IdMappingConfig>,
    pub gid_mappings: Vec<IdMappingConfig>,
    pub masked_paths: Vec<AbsolutePath>,
    pub readonly_paths: Vec<AbsolutePath>,
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

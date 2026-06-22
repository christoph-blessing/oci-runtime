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

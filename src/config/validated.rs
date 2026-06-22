use super::validation::AbsolutePath;
use super::validation::ExistingDir;
use caps::CapsHashSet;
use nix::mount::MsFlags;
use semver::Version;

#[derive(Debug)]
pub struct Config {
    pub oci_version: Version,
    pub hostname: Option<String>,
    pub root: RootConfig,
    pub mounts: Vec<MountConfig>,
    pub process: Option<ProcessConfig>,
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
    pub capabilities: Option<CapabilitiesConfig>,
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

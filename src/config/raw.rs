use caps::{CapSet, CapsHashSet};
use nix::{mount::MsFlags, sched::CloneFlags, sys::resource::Resource};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub oci_version: String,
    pub hostname: Option<String>,
    pub root: RootConfig,
    pub mounts: Option<Vec<MountConfig>>,
    pub process: Option<ProcessConfig>,
    pub linux: LinuxConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RootConfig {
    pub path: PathBuf,
    pub readonly: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MountConfig {
    pub destination: PathBuf,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub source: Option<String>,
    pub options: Option<Vec<String>>,
}

impl MountConfig {
    pub fn flags(&self) -> MsFlags {
        let mut flags = MsFlags::empty();
        if let Some(options) = &self.options {
            for option in options {
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
        }
        flags
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessConfig {
    pub cwd: PathBuf,
    pub env: Option<Vec<String>>,
    pub args: Vec<String>,
    pub user: UserConfig,
    pub capabilities: Option<CapabilitiesConfig>,
    pub no_new_privileges: Option<bool>,
    pub rlimits: Option<Vec<RlimitConfig>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxConfig {
    pub namespaces: Vec<NamespaceConfig>,
    pub uid_mappings: Option<Vec<IdMappingConfig>>,
    pub gid_mappings: Option<Vec<IdMappingConfig>>,
    pub masked_paths: Option<Vec<PathBuf>>,
    pub readonly_paths: Option<Vec<PathBuf>>,
}

impl From<&LinuxConfig> for CloneFlags {
    fn from(value: &LinuxConfig) -> Self {
        let mut flags = CloneFlags::empty();
        for namespace in &value.namespaces {
            flags |= CloneFlags::from(&namespace.kind)
        }
        flags
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct UserConfig {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CapabilitiesConfig {
    pub effective: Option<CapsHashSet>,
    pub bounding: Option<CapsHashSet>,
    pub inheritable: Option<CapsHashSet>,
    pub permitted: Option<CapsHashSet>,
    pub ambient: Option<CapsHashSet>,
}

impl CapabilitiesConfig {
    pub fn get(&self, cset: CapSet) -> Option<&CapsHashSet> {
        let capabilities = match cset {
            CapSet::Ambient => &self.ambient,
            CapSet::Bounding => &self.bounding,
            CapSet::Effective => &self.effective,
            CapSet::Inheritable => &self.inheritable,
            CapSet::Permitted => &self.permitted,
        };
        capabilities.as_ref()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RlimitConfig {
    #[serde(rename = "type")]
    pub kind: RlimitKind,
    pub soft: u64,
    pub hard: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NamespaceConfig {
    #[serde(rename = "type")]
    kind: NamespaceKind,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IdMappingConfig {
    #[serde(rename = "containerID")]
    pub container_id: usize,
    #[serde(rename = "hostID")]
    pub host_id: usize,
    pub size: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub enum RlimitKind {
    #[serde(rename = "RLIMIT_AS")]
    As,
    #[serde(rename = "RLIMIT_CORE")]
    Core,
    #[serde(rename = "RLIMIT_CPU")]
    Cpu,
    #[serde(rename = "RLIMIT_DATA")]
    Data,
    #[serde(rename = "RLIMIT_FSIZE")]
    Fsize,
    #[serde(rename = "RLIMIT_LOCKS")]
    Locks,
    #[serde(rename = "RLIMIT_MEMLOCK")]
    Memlock,
    #[serde(rename = "RLIMIT_MSGQUEUE")]
    Msgqueue,
    #[serde(rename = "RLIMIT_NICE")]
    Nice,
    #[serde(rename = "RLIMIT_NOFILE")]
    Nofile,
    #[serde(rename = "RLIMIT_NPROC")]
    Nproc,
    #[serde(rename = "RLIMIT_RSS")]
    Rss,
    #[serde(rename = "RLIMIT_RTPRIO")]
    Rtprio,
    #[serde(rename = "RLIMIT_RTTIME")]
    Rttime,
    #[serde(rename = "RLIMIT_SIGPENDING")]
    Sigpending,
    #[serde(rename = "RLIMIT_STACK")]
    Stack,
}

impl From<&RlimitKind> for Resource {
    fn from(value: &RlimitKind) -> Self {
        match value {
            RlimitKind::As => Resource::RLIMIT_AS,
            RlimitKind::Core => Resource::RLIMIT_CORE,
            RlimitKind::Cpu => Resource::RLIMIT_CPU,
            RlimitKind::Data => Resource::RLIMIT_DATA,
            RlimitKind::Fsize => Resource::RLIMIT_FSIZE,
            RlimitKind::Locks => Resource::RLIMIT_LOCKS,
            RlimitKind::Memlock => Resource::RLIMIT_MEMLOCK,
            RlimitKind::Msgqueue => Resource::RLIMIT_MSGQUEUE,
            RlimitKind::Nice => Resource::RLIMIT_NICE,
            RlimitKind::Nofile => Resource::RLIMIT_NOFILE,
            RlimitKind::Nproc => Resource::RLIMIT_NPROC,
            RlimitKind::Rss => Resource::RLIMIT_RSS,
            RlimitKind::Rtprio => Resource::RLIMIT_RTPRIO,
            RlimitKind::Rttime => Resource::RLIMIT_RTTIME,
            RlimitKind::Sigpending => Resource::RLIMIT_SIGPENDING,
            RlimitKind::Stack => Resource::RLIMIT_STACK,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum NamespaceKind {
    Pid,
    Network,
    Ipc,
    Uts,
    Mount,
    Cgroup,
    User,
}

impl From<&NamespaceKind> for CloneFlags {
    fn from(value: &NamespaceKind) -> Self {
        match value {
            NamespaceKind::Pid => CloneFlags::CLONE_NEWPID,
            NamespaceKind::Network => CloneFlags::CLONE_NEWNET,
            NamespaceKind::Ipc => CloneFlags::CLONE_NEWIPC,
            NamespaceKind::Uts => CloneFlags::CLONE_NEWUTS,
            NamespaceKind::Mount => CloneFlags::CLONE_NEWNS,
            NamespaceKind::Cgroup => CloneFlags::CLONE_NEWCGROUP,
            NamespaceKind::User => CloneFlags::CLONE_NEWUSER,
        }
    }
}

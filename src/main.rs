use std::ffi::{CStr, CString};
use std::fmt::Display;
use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::path::PathBuf;

use caps::errors::CapsError;
use caps::{CapSet, CapsHashSet};
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::prctl::set_no_new_privs;
use nix::sys::resource::{Resource, setrlimit};
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::{
    Gid, Uid, chdir, close, execve, pipe, pivot_root, read, setgid, sethostname, setuid, write,
};

const STACK_SIZE: usize = 1024 * 1024;

fn start_container(config: raw_config::Config) {
    let mut stack = vec![0u8; STACK_SIZE];
    let (read_fd, write_fd) = pipe().expect("failed to create pipe");
    let read_fd = read_fd.into_raw_fd();
    let write_fd = write_fd.into_raw_fd();

    let cb = Box::new(|| {
        close(write_fd).expect("failed to close write_fd in child");
        let mut buffer = vec![0u8; 1];
        let borrowed = unsafe { BorrowedFd::borrow_raw(read_fd) };
        read(borrowed, &mut buffer).expect("failed to read from pipe");
        close(read_fd).expect("failed to close read_fd in child");

        mount(
            None::<&str>,
            "/",
            None::<&str>,
            MsFlags::MS_PRIVATE | MsFlags::MS_REC,
            None::<&str>,
        )
        .expect("failed to make mounts private");

        let old_root = config.root.path.join("old_root");
        let old_root_after_pivot = "/old_root";
        mount(
            Some(&config.root.path),
            &config.root.path,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to bind mount new_root");

        if let Some(mounts) = &config.mounts {
            for mount_config in mounts {
                let destination = config.root.path.join(
                    &mount_config
                        .destination
                        .strip_prefix("/")
                        .unwrap_or_else(|e| {
                            panic!(
                                "failed to strip prefix from {}: {}",
                                mount_config.destination.display(),
                                e
                            )
                        }),
                );
                fs::create_dir_all(&destination).unwrap_or_else(|e| {
                    panic!("failed to create: {}: {}", destination.display(), e)
                });
                if mount_config.kind.as_deref() == Some("cgroup") {
                    mount(
                        Some(&mount_config.destination),
                        &destination,
                        None::<&str>,
                        MsFlags::MS_BIND | mount_config.flags(),
                        None::<&str>,
                    )
                    .expect("failed to mount cgroup");
                } else {
                    mount(
                        mount_config.source.as_deref(),
                        &destination,
                        mount_config.kind.as_deref(),
                        mount_config.flags(),
                        None::<&str>,
                    )
                    .unwrap_or_else(|e| {
                        panic!("failed to mount: {}: {}", destination.display(), e)
                    });
                }
            }
        }

        for path in ["dev/null", "dev/zero", "dev/urandom", "dev/tty"] {
            let destination = &config.root.path.join(path);
            fs::File::create(destination)
                .unwrap_or_else(|e| panic!("failed to create {}: {}", destination.display(), e));
            mount(
                Some(format!("/{}", path).as_str()),
                destination,
                None::<&str>,
                MsFlags::MS_BIND,
                None::<&str>,
            )
            .unwrap_or_else(|e| panic!("failed to mount {}: {}", destination.display(), e));
        }

        fs::create_dir(&old_root).expect("failed to create old_root");
        chdir("/").expect("failed to change current working directory");
        pivot_root(&config.root.path, &old_root).expect("failed to pivot root");

        umount2(old_root_after_pivot, MntFlags::MNT_DETACH).expect("failed to unmount old_root");
        fs::remove_dir(old_root_after_pivot).expect("failed to remove old_root");

        if let Some(hostname) = &config.hostname {
            sethostname(hostname).expect("failed to set hostname");
        }

        if config.root.readonly == Some(true) {
            mount(
                None::<&str>,
                "/",
                None::<&str>,
                MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY,
                None::<&str>,
            )
            .expect("failed to remount / as read only");
        }

        for path in config.linux.masked_paths.clone().unwrap_or_default() {
            let metadata = fs::metadata(&path);
            match metadata {
                Ok(m) if m.is_dir() => {
                    mount(
                        Some("tmpfs"),
                        &path,
                        Some("tmpfs"),
                        MsFlags::MS_RDONLY,
                        None::<&str>,
                    )
                    .unwrap_or_else(|e| panic!("failed to mask dir {}: {}", path.display(), e));
                }
                Ok(_) => {
                    mount(
                        Some("/dev/null"),
                        &path,
                        None::<&str>,
                        MsFlags::MS_BIND,
                        None::<&str>,
                    )
                    .unwrap_or_else(|e| panic!("failed to mask file {}: {}", path.display(), e));
                }
                Err(_) => {}
            }
        }

        if let Some(readonly_paths) = &config.linux.readonly_paths {
            for path in readonly_paths {
                mount(
                    Some(path),
                    path,
                    None::<&str>,
                    MsFlags::MS_BIND,
                    None::<&str>,
                )
                .unwrap_or_else(|e| panic!("failed to bind mount {}: {}", path.display(), e));
                mount(
                    Some(path),
                    path,
                    None::<&str>,
                    MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
                    None::<&str>,
                )
                .unwrap_or_else(|e| {
                    panic!("failed to remount as read only {}: {}", path.display(), e)
                });
            }
        }

        if let Some(process) = &config.process {
            // Change current working directory
            chdir(&process.cwd)
                .unwrap_or_else(|e| panic!("failed to chdir to {}: {}", process.cwd.display(), e));

            // Create envp and get PATH environment variable
            let mut env_c: Vec<CString> = Vec::new();
            let mut maybe_path_env: Option<&str> = None;
            if let Some(env) = &process.env {
                maybe_path_env = env
                    .iter()
                    .find(|&s| s.starts_with("PATH="))
                    .map(|s| &s[5..]);
                env_c = env
                    .iter()
                    .map(|s| {
                        CString::new(s.as_str()).unwrap_or_else(|e| {
                            panic!("failed to create C string from {}: {}", s, e)
                        })
                    })
                    .collect();
            }
            let envp: Vec<&CStr> = env_c.iter().map(|s| s.as_c_str()).collect();

            // Create argv and try to resolve absolute path to executable
            let mut args = process.args.clone();
            if let Some(path_env) = maybe_path_env {
                if let Some(absolute_file) = path_env
                    .split(":")
                    .map(|s| {
                        let mut p = PathBuf::from(s);
                        p.push(&args[0]);
                        p
                    })
                    .find(|p| p.exists())
                {
                    args[0] = absolute_file
                        .to_str()
                        .expect("failed to convert path to string")
                        .to_owned();
                }
            }
            let args_c: Vec<CString> = args
                .iter()
                .map(|a| {
                    CString::new(a.as_str())
                        .unwrap_or_else(|e| panic!("failed to create C string from {}: {}", a, e))
                })
                .collect();
            let argv: Vec<&CStr> = args_c.iter().map(|a| a.as_c_str()).collect();

            // Set user and group ids
            setgid(Gid::from_raw(process.user.gid)).expect("failed to setgid");
            setuid(Uid::from_raw(process.user.uid)).expect("failed to setuid");

            // Set resource limits
            if let Some(rlimits) = &process.rlimits {
                for rlimit in rlimits {
                    let resource = Resource::from(&rlimit.kind);
                    setrlimit(resource, rlimit.soft, rlimit.hard)
                        .unwrap_or_else(|e| panic!("failed to set rlimit {:?}: {}", resource, e));
                }
            }

            // Set capabilities
            if let Some(cap_config) = &process.capabilities {
                let existing_bounding = caps::read(None, CapSet::Bounding)
                    .expect("failed to read bounding capabilities");
                let new_bounding = cap_config.bounding.clone().unwrap_or_default();
                for capability in existing_bounding.difference(&new_bounding) {
                    caps::drop(None, CapSet::Bounding, *capability).unwrap_or_else(|e| {
                        panic!("failed to drop bounding capability {}: {}", capability, e)
                    });
                }

                for cset in [
                    CapSet::Inheritable,
                    CapSet::Effective,
                    CapSet::Permitted,
                    CapSet::Ambient,
                ] {
                    set_capabilities(cset, cap_config.get(cset))
                        .unwrap_or_else(|e| panic!("failed to set cap set {:?}: {}", cset, e));
                }
            }

            // Prevent process from getting new privileges
            if process.no_new_privileges == Some(true) {
                set_no_new_privs().expect("failed to set no_new_privs");
            }

            // Replace current process
            execve(argv[0], argv.as_slice(), envp.as_slice())
                .expect("failed to replace current process");
        }
        0
    });

    let pid = unsafe {
        clone(
            cb,
            &mut stack,
            CloneFlags::from(&config.linux),
            Some(Signal::SIGCHLD as i32),
        )
    }
    .expect("failed to clone process");
    close(read_fd).expect("failed to close read_fd in parent");

    if let Some(uid_mappings) = &config.linux.uid_mappings {
        fs::write(
            format!("/proc/{}/uid_map", pid),
            create_id_map_contents(uid_mappings),
        )
        .expect("failed to write to uid_map");
    }

    if let Some(gid_mappings) = &config.linux.gid_mappings {
        fs::write(format!("/proc/{}/setgroups", pid), "deny")
            .expect("failed to write to setgroups");
        fs::write(
            format!("/proc/{}/gid_map", pid),
            create_id_map_contents(gid_mappings),
        )
        .expect("failed to write to gid_map");
    }

    let mut buffer = vec![0u8; 1];
    let borrowed = unsafe { BorrowedFd::borrow_raw(write_fd) };
    write(borrowed, &mut buffer).expect("failed to write to pipe");
    close(write_fd).expect("failed to close write_fd in parent");

    waitpid(pid, None).expect("failed to wait for child");
}

fn set_capabilities(cset: CapSet, new_caps: Option<&CapsHashSet>) -> Result<(), CapsError> {
    match new_caps {
        Some(capabilities) => caps::set(None, cset, capabilities),
        None => caps::clear(None, cset),
    }
}

fn create_id_map_contents(mappings: &[raw_config::IdMappingConfig]) -> String {
    mappings
        .iter()
        .map(|m| format!("{} {} {}\n", m.container_id, m.host_id, m.size))
        .collect()
}

mod raw_config {
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
}

struct ValidatedConfig {
    oci_version: String,
}

enum ValidationError {
    InvalidVersion,
}

impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVersion => write!(f, "Invalid OCI version"),
        }
    }
}

fn validate(config: raw_config::Config) -> Result<ValidatedConfig, ValidationError> {
    if config.oci_version != "1.3.0" {
        return Err(ValidationError::InvalidVersion);
    }
    Ok(ValidatedConfig {
        oci_version: config.oci_version,
    })
}

fn main() {
    let bundle_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: oci-runtime <bundle_path>"),
    );
    let config_path = bundle_path.join("config.json");
    let config_string = fs::read_to_string(config_path).expect("failed to read config");
    let mut config: raw_config::Config =
        serde_json::from_str(&config_string).expect("failed to parse config");
    config.root.path = bundle_path.join(config.root.path);

    if let Some(mounts) = &mut config.mounts {
        for mount in mounts {
            mount.destination = PathBuf::from("/").join(mount.destination.clone());
        }
    }

    validate(config.clone()).unwrap_or_else(|e| panic!("failed to validate config: {}", e));

    start_container(config);
}

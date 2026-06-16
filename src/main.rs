use std::ffi::{CStr, CString};
use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::path::PathBuf;

use caps::errors::CapsError;
use caps::{CapSet, CapsHashSet};
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::prctl::set_no_new_privs;
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::{
    Gid, Uid, chdir, close, execve, pipe, pivot_root, read, setgid, sethostname, setuid, write,
};
use serde::Deserialize;

const STACK_SIZE: usize = 1024 * 1024;

fn start_container(config: Config) {
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

        if let Some(process) = &config.process {
            chdir(&process.cwd)
                .unwrap_or_else(|e| panic!("failed to chdir to {}: {}", process.cwd.display(), e));

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

            setgid(Gid::from_raw(process.user.gid)).expect("failed to setgid");
            setuid(Uid::from_raw(process.user.uid)).expect("failed to setuid");

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

            if process.no_new_privileges == Some(true) {
                set_no_new_privs().expect("failed to set no_new_privs");
            }

            execve(argv[0], argv.as_slice(), envp.as_slice())
                .expect("failed to replace current process");
        }
        0
    });

    let pid = unsafe {
        clone(
            cb,
            &mut stack,
            config.linux.clone_flags(),
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

fn create_id_map_contents(mappings: &[IdMappingConfig]) -> String {
    mappings
        .iter()
        .map(|m| format!("{} {} {}\n", m.container_id, m.host_id, m.size))
        .collect()
}

#[derive(Debug, Deserialize)]
struct RootConfig {
    path: PathBuf,
    readonly: Option<bool>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct NamespaceConfig {
    #[serde(rename = "type")]
    kind: NamespaceKind,
}

impl NamespaceConfig {
    fn flag(&self) -> CloneFlags {
        match self.kind {
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

#[derive(Debug, Deserialize)]
struct MountConfig {
    destination: PathBuf,
    #[serde(rename = "type")]
    kind: Option<String>,
    source: Option<String>,
    options: Option<Vec<String>>,
}

impl MountConfig {
    fn flags(&self) -> MsFlags {
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

#[derive(Debug, Deserialize)]
struct IdMappingConfig {
    #[serde(rename = "containerID")]
    container_id: usize,
    #[serde(rename = "hostID")]
    host_id: usize,
    size: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinuxConfig {
    namespaces: Vec<NamespaceConfig>,
    uid_mappings: Option<Vec<IdMappingConfig>>,
    gid_mappings: Option<Vec<IdMappingConfig>>,
    masked_paths: Option<Vec<PathBuf>>,
}

impl LinuxConfig {
    fn clone_flags(&self) -> CloneFlags {
        let mut flags = CloneFlags::empty();
        for namespace in &self.namespaces {
            flags |= namespace.flag();
        }
        flags
    }
}

#[derive(Debug, Deserialize)]
struct UserConfig {
    uid: u32,
    gid: u32,
}

#[derive(Debug, Deserialize)]
struct CapabilitiesConfig {
    effective: Option<CapsHashSet>,
    bounding: Option<CapsHashSet>,
    inheritable: Option<CapsHashSet>,
    permitted: Option<CapsHashSet>,
    ambient: Option<CapsHashSet>,
}

impl CapabilitiesConfig {
    fn get(&self, cset: CapSet) -> Option<&CapsHashSet> {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessConfig {
    cwd: PathBuf,
    env: Option<Vec<String>>,
    args: Vec<String>,
    user: UserConfig,
    capabilities: Option<CapabilitiesConfig>,
    no_new_privileges: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    oci_version: String,
    hostname: Option<String>,
    root: RootConfig,
    mounts: Option<Vec<MountConfig>>,
    process: Option<ProcessConfig>,
    linux: LinuxConfig,
}

fn main() {
    let bundle_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: oci-runtime <bundle_path>"),
    );
    let config_path = bundle_path.join("config.json");
    let config_string = fs::read_to_string(config_path).expect("failed to read config");
    let mut config: Config = serde_json::from_str(&config_string).expect("failed to parse config");
    config.root.path = bundle_path.join(config.root.path);

    if let Some(mounts) = &mut config.mounts {
        for mount in mounts {
            mount.destination = PathBuf::from("/").join(mount.destination.clone());
        }
    }

    start_container(config);
}

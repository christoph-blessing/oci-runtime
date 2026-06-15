use std::ffi::{CStr, CString};
use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::path::PathBuf;

use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::{chdir, close, execve, pipe, pivot_root, read, sethostname, write};
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

        let env: Vec<&CStr> = config
            .process
            .as_ref()
            .and_then(|p| p.env.as_deref())
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_c_str())
            .collect();
        execve::<&CStr, &CStr>(&c"/bin/sh", &[c"/bin/sh"], env.as_slice())
            .expect("failed to replace current process");
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
struct ProcessConfig {
    env: Option<Vec<CString>>,
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

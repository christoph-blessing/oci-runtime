use std::ffi::CStr;
use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::path::PathBuf;

use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::{
    chdir, close, execve, getgid, getuid, pipe, pivot_root, read, sethostname, write,
};
use serde::Deserialize;

const STACK_SIZE: usize = 1024 * 1024;

fn start_container(config: Config) {
    let mut flags = CloneFlags::empty();
    for namespace in config.linux.namespaces {
        let flag = match namespace.kind {
            NamespaceKind::Pid => CloneFlags::CLONE_NEWPID,
            NamespaceKind::Network => CloneFlags::CLONE_NEWNET,
            NamespaceKind::Ipc => CloneFlags::CLONE_NEWIPC,
            NamespaceKind::Uts => CloneFlags::CLONE_NEWUTS,
            NamespaceKind::Mount => CloneFlags::CLONE_NEWNS,
            NamespaceKind::Cgroup => CloneFlags::CLONE_NEWCGROUP,
            NamespaceKind::User => CloneFlags::CLONE_NEWUSER,
        };
        flags |= flag;
    }

    // TODO: Remove this once mounts are dynamic
    flags |= CloneFlags::CLONE_NEWNET;

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
        fs::create_dir(&old_root).expect("failed to create old_root");
        chdir("/").expect("failed to change current working directory");
        pivot_root(&config.root.path, &old_root).expect("failed to pivot root");

        mount(
            Some("proc"),
            "/proc",
            Some("proc"),
            MsFlags::MS_NODEV | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID,
            None::<&str>,
        )
        .expect("failed to mount proc");

        mount(
            Some("sysfs"),
            "/sys",
            Some("sysfs"),
            MsFlags::MS_NODEV | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID,
            None::<&str>,
        )
        .expect("failed to mount sysfs");

        fs::File::create("/dev/null").expect("failed to create /dev/null");
        mount(
            Some("/old_root/dev/null"),
            "/dev/null",
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to mount /dev/null");

        fs::File::create("/dev/zero").expect("failed to create /dev/zero");
        mount(
            Some("/old_root/dev/zero"),
            "/dev/zero",
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to mount /dev/zero");

        fs::File::create("/dev/urandom").expect("failed to create /dev/urandom");
        mount(
            Some("/old_root/dev/urandom"),
            "/dev/urandom",
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to mount /dev/urandom");

        fs::File::create("/dev/tty").expect("failed to create /dev/tty");
        mount(
            Some("/old_root/dev/tty"),
            "/dev/urandom",
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to mount /dev/tty");

        umount2(old_root_after_pivot, MntFlags::MNT_DETACH).expect("failed to unmount old_root");
        fs::remove_dir(old_root_after_pivot).expect("failed to remove old_root");

        sethostname("container").expect("failed to set hostname");

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

        execve::<&CStr, &CStr>(&c"/bin/sh", &[c"/bin/sh"], &[])
            .expect("failed to replace current process");
        0
    });

    let pid = unsafe { clone(cb, &mut stack, flags, Some(Signal::SIGCHLD as i32)) }
        .expect("failed to clone process");
    close(read_fd).expect("failed to close read_fd in parent");

    fs::write(
        format!("/proc/{}/uid_map", pid),
        format!("0 {} 1", getuid()),
    )
    .expect("failed to write to uid_map");
    fs::write(format!("/proc/{}/setgroups", pid), "deny").expect("failed to write to setgroups");
    fs::write(
        format!("/proc/{}/gid_map", pid),
        format!("0 {} 1", getgid()),
    )
    .expect("failed to write to gid_map");

    let mut buffer = vec![0u8; 1];
    let borrowed = unsafe { BorrowedFd::borrow_raw(write_fd) };
    write(borrowed, &mut buffer).expect("failed to write to pipe");
    close(write_fd).expect("failed to close write_fd in parent");

    waitpid(pid, None).expect("failed to wait for child");
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

#[derive(Debug, Deserialize)]
struct MountConfig {
    destination: PathBuf,
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
struct LinuxConfig {
    namespaces: Vec<NamespaceConfig>,
}

#[derive(Debug, Deserialize)]
struct Config {
    root: RootConfig,
    mounts: Option<Vec<MountConfig>>,
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

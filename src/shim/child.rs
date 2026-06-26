use std::{
    fs::File,
    io::{self, Read},
    os::fd::BorrowedFd,
    path::Path,
};

use nix::mount::{MntFlags, MsFlags};

use crate::config::validated::{AbsolutePath, Config, MountConfig, RootConfig};

pub fn run(
    config: &Config,
    read_mappings_ready_fd: i32,
    write_mappings_ready_fd: i32,
    start_fifo_path: &Path,
) -> Result<(), ChildError> {
    recv_mappings_ready_signal(read_mappings_ready_fd, write_mappings_ready_fd)?;
    make_mounts_private()?;
    let start_fifo = open_start_signal_fifo(start_fifo_path)?;
    pivot_root(&config.root, &config.mounts)?;
    if let Some(hostname) = &config.hostname {
        nix::unistd::sethostname(hostname)?;
    }
    if config.root.readonly {
        apply_root_readonly()?;
    }
    apply_masked_paths(&config.linux.masked_paths)?;
    apply_readonly_paths(&config.linux.readonly_paths)?;
    recv_start_signal(start_fifo)?;
    Ok(())
}

#[derive(Debug)]
pub enum ChildError {
    Io(io::Error),
    Syscall(nix::Error),
}

impl From<io::Error> for ChildError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<nix::Error> for ChildError {
    fn from(value: nix::Error) -> Self {
        Self::Syscall(value)
    }
}

fn recv_mappings_ready_signal(read_fd: i32, write_fd: i32) -> Result<(), ChildError> {
    nix::unistd::close(write_fd)?;
    let mut buffer = vec![0u8; 1];
    let borrowed = unsafe { BorrowedFd::borrow_raw(read_fd) };
    nix::unistd::read(borrowed, &mut buffer)?;
    nix::unistd::close(read_fd)?;
    Ok(())
}

fn open_start_signal_fifo(start_fifo_path: &Path) -> Result<File, ChildError> {
    Ok(File::open(&start_fifo_path)?)
}

fn make_mounts_private() -> Result<(), nix::Error> {
    nix::mount::mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_PRIVATE | MsFlags::MS_REC,
        None::<&str>,
    )?;
    Ok(())
}

fn pivot_root(root_config: &RootConfig, mounts: &Vec<MountConfig>) -> Result<(), ChildError> {
    let root_path = root_config.path.as_path();
    let old_root = root_path.join("old_root");
    let old_root_after_pivot = "/old_root";
    nix::mount::mount(
        Some(root_path),
        root_path,
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    )?;

    for mount_config in mounts {
        let destination = root_path.join(
            &mount_config
                .destination
                .as_path()
                .strip_prefix("/")
                .unwrap_or_else(|e| {
                    panic!(
                        "failed to strip prefix from {}: {}",
                        mount_config.destination.as_path().display(),
                        e
                    )
                }),
        );
        std::fs::create_dir_all(&destination)?;
        if mount_config.kind.as_deref() == Some("cgroup") {
            nix::mount::mount(
                Some(mount_config.destination.as_path()),
                &destination,
                None::<&str>,
                MsFlags::MS_BIND | mount_config.flags,
                None::<&str>,
            )?;
        } else {
            nix::mount::mount(
                mount_config.source.as_deref(),
                &destination,
                mount_config.kind.as_deref(),
                mount_config.flags,
                None::<&str>,
            )?;
        }
    }

    for path in ["dev/null", "dev/zero", "dev/urandom", "dev/tty"] {
        let destination = &root_path.join(path);
        std::fs::File::create(destination)?;
        nix::mount::mount(
            Some(format!("/{}", path).as_str()),
            destination,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )?;
    }

    std::fs::create_dir(&old_root)?;
    nix::unistd::chdir("/")?;
    nix::unistd::pivot_root(root_path, &old_root)?;

    nix::mount::umount2(old_root_after_pivot, MntFlags::MNT_DETACH)?;
    std::fs::remove_dir(old_root_after_pivot)?;
    Ok(())
}

fn apply_root_readonly() -> Result<(), ChildError> {
    nix::mount::mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY,
        None::<&str>,
    )?;
    Ok(())
}

fn apply_masked_paths(masked_paths: &Vec<AbsolutePath>) -> Result<(), ChildError> {
    for path in masked_paths {
        let metadata = std::fs::metadata(path.as_path());
        match metadata {
            Ok(m) if m.is_dir() => {
                nix::mount::mount(
                    Some("tmpfs"),
                    path.as_path(),
                    Some("tmpfs"),
                    MsFlags::MS_RDONLY,
                    None::<&str>,
                )?;
            }
            Ok(_) => {
                nix::mount::mount(
                    Some("/dev/null"),
                    path.as_path(),
                    None::<&str>,
                    MsFlags::MS_BIND,
                    None::<&str>,
                )?;
            }
            Err(_) => {}
        }
    }
    Ok(())
}

fn apply_readonly_paths(readonly_paths: &Vec<AbsolutePath>) -> Result<(), ChildError> {
    for path in readonly_paths {
        nix::mount::mount(
            Some(path.as_path()),
            path.as_path(),
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )?;
        nix::mount::mount(
            Some(path.as_path()),
            path.as_path(),
            None::<&str>,
            MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
            None::<&str>,
        )?;
    }
    Ok(())
}

fn recv_start_signal(mut start_fifo: File) -> Result<(), ChildError> {
    let mut buffer = [0u8; 1];
    start_fifo.read_exact(&mut buffer)?;
    Ok(())
}

use std::{
    ffi::{CStr, CString, NulError},
    fs::File,
    io::{self, Read},
    os::fd::BorrowedFd,
    path::{Path, PathBuf},
};

use caps::{CapSet, errors::CapsError};
use nix::{
    mount::{MntFlags, MsFlags},
    unistd::{Gid, Uid},
};

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

    // Change current working directory
    nix::unistd::chdir(config.process.cwd.as_path())?;
    let env_c: Vec<CString> = config
        .process
        .env
        .iter()
        .map(|s| CString::new(s.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    let envp: Vec<&CStr> = env_c.iter().map(|s| s.as_c_str()).collect();

    // Create argv and try to resolve absolute path to executable
    let maybe_path_env = config
        .process
        .env
        .iter()
        .find(|&s| s.starts_with("PATH="))
        .map(|s| &s[5..]);
    let mut args = config.process.args.clone();
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
            args[0] = absolute_file.to_string_lossy().into_owned();
        }
    }
    let args_c: Vec<CString> = args
        .iter()
        .map(|a| CString::new(a.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    let argv: Vec<&CStr> = args_c.iter().map(|a| a.as_c_str()).collect();

    nix::unistd::setgid(Gid::from_raw(config.process.user.gid))?;
    nix::unistd::setuid(Uid::from_raw(config.process.user.uid))?;
    for rlimit in &config.process.rlimits {
        nix::sys::resource::setrlimit(rlimit.resource, rlimit.soft, rlimit.hard)?;
    }
    apply_capabilities(config)?;
    if config.process.no_new_privileges {
        nix::sys::prctl::set_no_new_privs()?;
    }
    recv_start_signal(start_fifo)?;
    nix::unistd::execve(argv[0], argv.as_slice(), envp.as_slice())?;
    Ok(())
}

#[derive(Debug)]
pub enum ChildError {
    Io(io::Error),
    Syscall(nix::Error),
    NulByte(NulError),
    Capabilities(CapsError),
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

impl From<NulError> for ChildError {
    fn from(value: NulError) -> Self {
        Self::NulByte(value)
    }
}

impl From<CapsError> for ChildError {
    fn from(value: CapsError) -> Self {
        Self::Capabilities(value)
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

fn apply_capabilities(config: &Config) -> Result<(), ChildError> {
    let existing_bounding = caps::read(None, CapSet::Bounding)?;
    let new_bounding = &config.process.capabilities.bounding;
    for capability in existing_bounding.difference(&new_bounding) {
        caps::drop(None, CapSet::Bounding, *capability)?;
    }

    for cset in [
        CapSet::Inheritable,
        CapSet::Effective,
        CapSet::Permitted,
        CapSet::Ambient,
    ] {
        let capabilities = match cset {
            CapSet::Ambient => &config.process.capabilities.ambient,
            CapSet::Bounding => &config.process.capabilities.bounding,
            CapSet::Effective => &config.process.capabilities.effective,
            CapSet::Inheritable => &config.process.capabilities.inheritable,
            CapSet::Permitted => &config.process.capabilities.permitted,
        };
        caps::set(None, cset, capabilities)?;
    }
    Ok(())
}

fn recv_start_signal(mut start_fifo: File) -> Result<(), ChildError> {
    let mut buffer = [0u8; 1];
    start_fifo.read_exact(&mut buffer)?;
    Ok(())
}

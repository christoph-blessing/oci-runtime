use std::ffi::{CStr, CString};
use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::path::PathBuf;
use std::process::exit;

use caps::errors::CapsError;
use caps::{CapSet, CapsHashSet};
use config::raw::{Config, IdMappingConfig};
use config::validated::Config as ValidatedConfig;
use config::validation;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::prctl::set_no_new_privs;
use nix::sys::resource::{Resource, setrlimit};
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::{
    Gid, Uid, chdir, close, execve, pipe, pivot_root, read, setgid, sethostname, setuid, write,
};

mod config;

const STACK_SIZE: usize = 1024 * 1024;

fn start_container(raw_config: Config, validated_config: ValidatedConfig) {
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

        let root_path = validated_config.root.path.as_path();
        let old_root = root_path.join("old_root");
        let old_root_after_pivot = "/old_root";
        mount(
            Some(root_path),
            root_path,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to bind mount new_root");

        if let Some(mounts) = &raw_config.mounts {
            for mount_config in mounts {
                let destination =
                    root_path.join(&mount_config.destination.strip_prefix("/").unwrap_or_else(
                        |e| {
                            panic!(
                                "failed to strip prefix from {}: {}",
                                mount_config.destination.display(),
                                e
                            )
                        },
                    ));
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
            let destination = &root_path.join(path);
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
        pivot_root(root_path, &old_root).expect("failed to pivot root");

        umount2(old_root_after_pivot, MntFlags::MNT_DETACH).expect("failed to unmount old_root");
        fs::remove_dir(old_root_after_pivot).expect("failed to remove old_root");

        if let Some(hostname) = &raw_config.hostname {
            sethostname(hostname).expect("failed to set hostname");
        }

        if validated_config.root.readonly {
            mount(
                None::<&str>,
                "/",
                None::<&str>,
                MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY,
                None::<&str>,
            )
            .expect("failed to remount / as read only");
        }

        for path in raw_config.linux.masked_paths.clone().unwrap_or_default() {
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

        if let Some(readonly_paths) = &raw_config.linux.readonly_paths {
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

        if let Some(process) = &raw_config.process {
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
            CloneFlags::from(&raw_config.linux),
            Some(Signal::SIGCHLD as i32),
        )
    }
    .expect("failed to clone process");
    close(read_fd).expect("failed to close read_fd in parent");

    if let Some(uid_mappings) = &raw_config.linux.uid_mappings {
        fs::write(
            format!("/proc/{}/uid_map", pid),
            create_id_map_contents(uid_mappings),
        )
        .expect("failed to write to uid_map");
    }

    if let Some(gid_mappings) = &raw_config.linux.gid_mappings {
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

    let validated_config = match validation::validate(config.clone()) {
        Ok(config) => config,
        Err(errors) => {
            for error in errors {
                eprintln!("{}", error);
            }
            exit(1);
        }
    };

    start_container(config, validated_config);
}

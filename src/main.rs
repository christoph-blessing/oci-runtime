use std::ffi::CStr;
use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};

use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::{
    chdir, close, execve, getgid, getuid, pipe, pivot_root, read, sethostname, write,
};

const STACK_SIZE: usize = 1024 * 1024;

fn start_container() {
    let flags = CloneFlags::CLONE_NEWPID
        | CloneFlags::CLONE_NEWCGROUP
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWNET
        | CloneFlags::CLONE_NEWNS
        | CloneFlags::CLONE_NEWUSER
        | CloneFlags::CLONE_NEWUTS;
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

        let new_root = "/tmp/rootfs";
        let old_root = "/tmp/rootfs/old_root";
        let old_root_after_pivot = "/old_root";
        mount(
            Some(new_root),
            new_root,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .expect("failed to bind mount new_root");
        fs::create_dir(old_root).expect("failed to create old_root");
        chdir("/").expect("failed to change current working directory");
        pivot_root(new_root, old_root).expect("failed to pivot root");

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

fn main() {}

use std::fs;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::{thread::sleep, time::Duration};

use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::unistd::{chdir, close, getgid, getuid, pipe, pivot_root, read, write};

const STACK_SIZE: usize = 1024 * 1024;

fn main() {
    let flags = CloneFlags::CLONE_NEWPID
        | CloneFlags::CLONE_NEWCGROUP
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWNET
        | CloneFlags::CLONE_NEWNS
        | CloneFlags::CLONE_NEWUSER
        | CloneFlags::CLONE_NEWUTS;
    let signal = Option::None;
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

        umount2(old_root_after_pivot, MntFlags::MNT_DETACH).expect("failed to unmount old_root");
        fs::remove_dir(old_root_after_pivot).expect("failed to remove old_root");

        loop {
            sleep(Duration::from_secs(1));
        }
    });

    let pid = unsafe { clone(cb, &mut stack, flags, signal) }.expect("failed to clone process");
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

    println!("{pid}")
}

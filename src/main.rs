use std::fs::{create_dir, remove_dir};
use std::{thread::sleep, time::Duration};

use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, clone};
use nix::unistd::{chdir, pivot_root};

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
    let cb = Box::new(|| {
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
        create_dir(old_root).expect("failed to create old_root");
        chdir("/").expect("failed to change current working directory");
        pivot_root(new_root, old_root).expect("failed to pivot root");
        umount2(old_root_after_pivot, MntFlags::MNT_DETACH).expect("failed to unmount old_root");
        remove_dir(old_root_after_pivot).expect("failed to remove old_root");

        loop {
            sleep(Duration::from_secs(1));
        }
    });

    let pid = unsafe { clone(cb, &mut stack, flags, signal) }.expect("failed to clone process");

    println!("{pid}")
}

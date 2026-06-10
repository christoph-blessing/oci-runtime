use std::{thread::sleep, time::Duration};

use nix::mount::{MsFlags, mount};
use nix::sched::{CloneFlags, clone};

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

        loop {
            sleep(Duration::from_secs(1));
        }
    });

    let pid = unsafe { clone(cb, &mut stack, flags, signal) }.expect("failed to clone process");

    println!("{pid}")
}

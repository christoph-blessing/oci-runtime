use std::{os::fd::BorrowedFd, path::PathBuf};

use crate::state::State;

pub fn run(id: &str, ready_fd: i32) {
    let state = State::load(id).expect("failed to load state in shim");
    state
        .finish_setup(42, PathBuf::from("/foo"))
        .expect("failed to finish setup");

    let mut buffer = [0u8; 1];
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &mut buffer)
        .expect("failed to send shim ready signal");
}

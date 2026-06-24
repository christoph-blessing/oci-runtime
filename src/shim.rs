use std::os::fd::BorrowedFd;

use crate::state::{State, Status};

pub fn run(id: &str, ready_fd: i32) {
    let mut state = State::load(id).expect("failed to load state in shim");
    state.status = Status::Created;
    state.pid = Some(42);
    state.save().expect("failed to save state in shim");

    let mut buffer = [0u8; 1];
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &mut buffer)
        .expect("failed to send shim ready signal");
}

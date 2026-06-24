use std::os::fd::BorrowedFd;

use nix::sys::stat::Mode;

use crate::state::State;

pub fn run(id: &str, ready_fd: i32) {
    let state = State::load(id).expect("failed to load state in shim");

    let start_fifo_path = state.state_dir().join("start.fifo");
    nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU).expect("failed to create start fifo");

    state
        .finish_setup(42, start_fifo_path)
        .expect("failed to finish setup");

    let mut buffer = [0u8; 1];
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &mut buffer)
        .expect("failed to send shim ready signal");
}

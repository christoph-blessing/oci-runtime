use std::{
    fs::{self, File},
    io::Read,
    os::fd::BorrowedFd,
};

use nix::sys::stat::Mode;

use crate::state::State;

pub fn run(id: &str, ready_fd: i32) {
    let state = State::load(id).expect("failed to load state in shim");

    let start_fifo_path = state.state_dir().join("start.fifo");
    nix::unistd::mkfifo(&start_fifo_path, Mode::S_IRWXU)
        .expect("failed to create start signal fifo");

    state
        .finish_setup(42, start_fifo_path.as_path())
        .expect("failed to finish setup");

    let mut buffer = [0u8; 1];
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &mut buffer)
        .expect("failed to send shim ready signal");

    let mut start_fifo = File::open(&start_fifo_path).expect("failed to open start signal fifo");
    let mut buffer = [0u8; 1];
    start_fifo
        .read_exact(&mut buffer)
        .expect("failed to read start signal");
    fs::remove_file(&start_fifo_path).expect("failed to remove start signal fifo");
}

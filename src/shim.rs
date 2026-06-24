use std::os::fd::BorrowedFd;

pub fn run(ready_fd: i32) {
    let mut buffer = [0u8; 1];
    nix::unistd::write(unsafe { BorrowedFd::borrow_raw(ready_fd) }, &mut buffer)
        .expect("failed to send shim ready signal");
}

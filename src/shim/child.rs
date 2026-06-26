use std::{
    fs::File,
    io::{self, Read},
    os::fd::BorrowedFd,
    path::Path,
};

use nix::mount::MsFlags;

use crate::config::validated::Config;

pub fn run(
    config: &Config,
    read_mappings_ready_fd: i32,
    write_mappings_ready_fd: i32,
    start_fifo_path: &Path,
) -> Result<(), ChildError> {
    recv_mappings_ready_signal(read_mappings_ready_fd, write_mappings_ready_fd)?;
    make_mounts_private()?;
    recv_start_signal(start_fifo_path)?;
    Ok(())
}

#[derive(Debug)]
pub enum ChildError {
    Io(io::Error),
    Syscall(nix::Error),
}

impl From<io::Error> for ChildError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<nix::Error> for ChildError {
    fn from(value: nix::Error) -> Self {
        Self::Syscall(value)
    }
}

fn recv_mappings_ready_signal(read_fd: i32, write_fd: i32) -> Result<(), ChildError> {
    nix::unistd::close(write_fd)?;
    let mut buffer = vec![0u8; 1];
    let borrowed = unsafe { BorrowedFd::borrow_raw(read_fd) };
    nix::unistd::read(borrowed, &mut buffer)?;
    nix::unistd::close(read_fd)?;
    Ok(())
}

fn make_mounts_private() -> Result<(), nix::Error> {
    nix::mount::mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_PRIVATE | MsFlags::MS_REC,
        None::<&str>,
    )?;
    Ok(())
}

fn recv_start_signal(start_fifo_path: &Path) -> Result<(), ChildError> {
    let mut start_fifo = File::open(&start_fifo_path)?;
    let mut buffer = [0u8; 1];
    start_fifo.read_exact(&mut buffer)?;
    Ok(())
}

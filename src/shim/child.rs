use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use nix::mount::MsFlags;

use crate::config::validated::Config;

pub fn run(config: &Config, start_fifo_path: &Path) -> Result<(), ChildError> {
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

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use crate::config::validated::Config;

pub fn run(config: &Config, start_fifo_path: &Path) -> Result<(), ChildError> {
    recv_start_signal(start_fifo_path)?;
    Ok(())
}

#[derive(Debug)]
pub enum ChildError {
    Fifo(FifoError),
}

impl From<FifoError> for ChildError {
    fn from(value: FifoError) -> Self {
        Self::Fifo(value)
    }
}

#[derive(Debug)]
pub struct FifoError(io::Error);

impl From<io::Error> for FifoError {
    fn from(value: io::Error) -> Self {
        Self(value)
    }
}

fn recv_start_signal(start_fifo_path: &Path) -> Result<(), FifoError> {
    let mut start_fifo = File::open(&start_fifo_path)?;
    let mut buffer = [0u8; 1];
    start_fifo.read_exact(&mut buffer)?;
    std::fs::remove_file(&start_fifo_path)?;
    Ok(())
}

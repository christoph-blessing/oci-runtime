use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
};

use crate::state::{Created, StateError, persist};

pub fn run(id: &str) -> Result<(), StartError> {
    let created = Created::load(id)?;
    send_start_signal(&created.internal.start_signal)?;
    let running = created.start()?;
    persist(&running.into())?;
    Ok(())
}

fn send_start_signal(path: &Path) -> Result<(), StartError> {
    let mut start_fifo = OpenOptions::new().write(true).open(path)?;
    let mut buffer = [0u8; 1];
    start_fifo.write(&mut buffer)?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[derive(Debug)]
pub enum StartError {
    State(StateError),
    Io(io::Error),
}

impl From<StateError> for StartError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<io::Error> for StartError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

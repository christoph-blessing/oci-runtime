use crate::state::{Created, StateError};

pub fn run(id: &str) -> Result<(), StartError> {
    let created = Created::load(id)?;
    created.start()?;
    Ok(())
}

#[derive(Debug)]
pub enum StartError {
    State(StateError),
}

impl From<StateError> for StartError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

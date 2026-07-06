use crate::state::{State, StateError};

pub fn run(id: &str) -> Result<State, StateError> {
    let state = crate::state::load(id)?;
    Ok(state)
}

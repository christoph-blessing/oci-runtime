use crate::{
    cmd::{create::CreateError, delete::DeleteError, kill::KillError, start::StartError},
    shim::ShimError,
    state::StateError,
};
use clap::{Parser, Subcommand};
use std::{error::Error, fmt::Display, path::PathBuf};

pub fn run() -> Result<(), CliError> {
    let result = dispatch();
    match result {
        Ok(_) => {}
        Err(CliError::Shim(_)) => {}
        Err(ref e) => {
            eprintln!("error: {}", e);
            let mut source = e.source();
            while let Some(error) = source {
                eprintln!("  caused by: {}", error);
                source = error.source()
            }
        }
    }
    result
}

fn dispatch() -> Result<(), CliError> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Create {
            container_id,
            bundle_path,
        } => {
            crate::cmd::create::run(container_id, bundle_path)?;
            println!("created container: {}", container_id)
        }
        Commands::Start { container_id } => {
            crate::cmd::start::run(container_id)?;
            println!("started container: {}", container_id)
        }
        Commands::Kill {
            container_id,
            signal,
        } => {
            crate::cmd::kill::run(container_id, signal)?;
            println!("killed container: {}", container_id)
        }
        Commands::Delete { container_id } => {
            crate::cmd::delete::run(container_id)?;
            println!("deleted container: {}", container_id)
        }
        Commands::State { container_id } => {
            let state = crate::cmd::state::run(container_id)?;
            println!("{:?}", state)
        }
        Commands::Shim {
            container_id,
            bundle_path,
            done_fd,
        } => crate::shim::run(container_id, bundle_path, *done_fd)?,
    };
    Ok(())
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Create {
        container_id: String,
        bundle_path: PathBuf,
    },
    Start {
        container_id: String,
    },
    Kill {
        container_id: String,
        signal: String,
    },
    Delete {
        container_id: String,
    },
    State {
        container_id: String,
    },
    #[command(hide = true, name = "__shim")]
    Shim {
        container_id: String,
        bundle_path: PathBuf,
        done_fd: i32,
    },
}

#[derive(Debug)]
pub enum CliError {
    Create(CreateError),
    Start(StartError),
    Kill(KillError),
    Delete(DeleteError),
    State(StateError),
    Shim(ShimError),
}

impl From<CreateError> for CliError {
    fn from(value: CreateError) -> Self {
        Self::Create(value)
    }
}

impl From<StartError> for CliError {
    fn from(value: StartError) -> Self {
        CliError::Start(value)
    }
}

impl From<KillError> for CliError {
    fn from(value: KillError) -> Self {
        Self::Kill(value)
    }
}

impl From<ShimError> for CliError {
    fn from(value: ShimError) -> Self {
        Self::Shim(value)
    }
}

impl From<DeleteError> for CliError {
    fn from(value: DeleteError) -> Self {
        Self::Delete(value)
    }
}

impl From<StateError> for CliError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create(_) => write!(f, "failed to create container"),
            Self::Start(_) => write!(f, "failed to start container"),
            Self::Kill(_) => write!(f, "failed to kill container"),
            Self::Delete(_) => write!(f, "failed to delete container"),
            Self::State(_) => write!(f, "failed to query state"),
            Self::Shim(_) => write!(f, "failed to run shim"),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Create(e) => Some(e),
            Self::Start(e) => Some(e),
            Self::Kill(e) => Some(e),
            Self::Delete(e) => Some(e),
            Self::State(e) => Some(e),
            Self::Shim(e) => Some(e),
        }
    }
}

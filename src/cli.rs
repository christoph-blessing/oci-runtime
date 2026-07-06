use crate::{
    cmd::create::CreateError, cmd::delete::DeleteError, cmd::kill::KillError,
    cmd::start::StartError, shim::ShimError,
};
use clap::{Parser, Subcommand};
use std::{error::Error, fmt::Display, path::PathBuf};

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let result: Result<(), CliError> = match &cli.command {
        Commands::Create {
            container_id,
            bundle_path,
        } => crate::cmd::create::run(container_id, bundle_path).map_err(|e| e.into()),
        Commands::Start { container_id } => {
            crate::cmd::start::run(container_id).map_err(|e| e.into())
        }
        Commands::Kill {
            container_id,
            signal,
        } => crate::cmd::kill::run(container_id, signal).map_err(|e| e.into()),
        Commands::Shim {
            container_id,
            bundle_path,
            done_fd,
        } => crate::shim::run(container_id, bundle_path, *done_fd).map_err(|e| e.into()),
        Commands::Delete { container_id } => {
            crate::cmd::delete::run(container_id).map_err(|e| e.into())
        }
    };
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

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create(_) => write!(f, "failed to create container"),
            Self::Start(_) => write!(f, "failed to start container"),
            Self::Kill(_) => write!(f, "failed to kill container"),
            Self::Delete(_) => write!(f, "failed to delete container"),
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
            Self::Shim(e) => Some(e),
        }
    }
}

use crate::{
    config::error::ConfigError, create::CreateError, shim::ShimError, start::StartError,
    state::StateError,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let result: Result<(), CliError> = match &cli.command {
        Commands::Create {
            container_id,
            bundle_path,
        } => crate::create::run(container_id, bundle_path).map_err(|e| e.into()),
        Commands::Start { container_id } => crate::start::run(container_id).map_err(|e| e.into()),
        Commands::Legacy { bundle_path } => crate::legacy::main(bundle_path).map_err(|e| e.into()),
        Commands::Shim {
            container_id,
            bundle_path,
            done_fd,
        } => crate::shim::run(container_id, bundle_path, *done_fd).map_err(|e| e.into()),
    };
    match result {
        Ok(_) => std::process::exit(0),

        Err(CliError::Create(CreateError::State(StateError::NotFound(ref id)))) => {
            eprintln!("container not found: {}", id);
        }
        Err(CliError::Create(CreateError::ShimExitedEarly)) => {
            eprintln!("shim exited without becoming ready");
        }
        Err(CliError::Create(CreateError::ShimReportedFailure)) => {
            eprintln!("shim reported failure during setup");
        }
        Err(CliError::Create(CreateError::State(StateError::InvalidState { ref state }))) => {
            eprintln!("cannot create container in state {}", state)
        }

        Err(CliError::Start(StartError::State(StateError::NotFound(ref id)))) => {
            eprintln!("container not found: {}", id);
        }
        Err(CliError::Start(StartError::State(StateError::InvalidState { ref state }))) => {
            eprintln!("cannot start container in state {}", state)
        }

        Err(CliError::Shim(ShimError::State(StateError::Config(ConfigError::NotFound(
            ref path,
        ))))) => {
            eprintln!("config not found: {}", path.display());
        }
        Err(CliError::Shim(ShimError::State(StateError::AlreadyExists(ref id)))) => {
            eprintln!("container already exists: {}", id);
        }
        Err(CliError::Shim(ShimError::State(StateError::Config(ConfigError::Parse(
            ref error,
        ))))) => {
            eprintln!("failed to parse config: {}", error)
        }
        Err(_) => {
            println!("{:?}", result);
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
    Legacy {
        bundle_path: PathBuf,
    },
    Create {
        container_id: String,
        bundle_path: PathBuf,
    },
    Start {
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

impl From<ShimError> for CliError {
    fn from(value: ShimError) -> Self {
        Self::Shim(value)
    }
}

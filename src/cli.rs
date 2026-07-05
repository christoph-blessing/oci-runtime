use crate::{create::CreateError, kill::KillError, shim::ShimError, start::StartError};
use clap::{Parser, Subcommand};
use std::{fmt::Display, path::PathBuf};

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let result: Result<(), CliError> = match &cli.command {
        Commands::Create {
            container_id,
            bundle_path,
        } => crate::create::run(container_id, bundle_path).map_err(|e| e.into()),
        Commands::Start { container_id } => crate::start::run(container_id).map_err(|e| e.into()),
        Commands::Legacy { bundle_path } => crate::legacy::main(bundle_path).map_err(|e| e.into()),
        Commands::Kill {
            container_id,
            signal,
        } => crate::kill::run(container_id, signal).map_err(|e| e.into()),
        Commands::Shim {
            container_id,
            bundle_path,
            done_fd,
        } => crate::shim::run(container_id, bundle_path, *done_fd).map_err(|e| e.into()),
    };
    match result {
        Ok(_) => std::process::exit(0),

        Err(CliError::Shim(_)) => {}
        Err(ref e) => eprintln!("Error: {}", e),
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
    Kill {
        container_id: String,
        signal: String,
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

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create(e) => write!(f, "create error: {}", e),
            Self::Start(e) => write!(f, "start error: {}", e),
            Self::Kill(e) => write!(f, "kill error: {}", e),
            Self::Shim(e) => write!(f, "shim error: {}", e),
        }
    }
}

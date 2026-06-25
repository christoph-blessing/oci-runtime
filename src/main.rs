use crate::{config::error::ConfigError, create::CreateError, shim::ShimError, state::StateError};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod create;
mod legacy;
mod shim;
mod state;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
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
    #[command(hide = true, name = "__shim")]
    Shim {
        container_id: String,
        bundle_path: PathBuf,
        ready_fd: i32,
    },
}

#[derive(Debug)]
enum CliError {
    Create(CreateError),
    Shim(ShimError),
}

impl From<CreateError> for CliError {
    fn from(value: CreateError) -> Self {
        Self::Create(value)
    }
}

impl From<ShimError> for CliError {
    fn from(value: ShimError) -> Self {
        Self::Shim(value)
    }
}

fn main() {
    let cli = Cli::parse();
    let result: Result<(), CliError> = match &cli.command {
        Commands::Create {
            container_id,
            bundle_path,
        } => create::run(container_id, bundle_path).map_err(|e| e.into()),
        Commands::Legacy { bundle_path } => legacy::main(bundle_path).map_err(|e| e.into()),
        Commands::Shim {
            container_id,
            bundle_path,
            ready_fd,
        } => shim::run(container_id, bundle_path, *ready_fd).map_err(|e| e.into()),
    };
    match result {
        Ok(_) => std::process::exit(0),
        Err(CliError::Shim(ShimError::State(StateError::AlreadyExists(id)))) => {
            eprintln!("container already exists: {}", id);
            std::process::exit(1);
        }
        Err(CliError::Create(CreateError::State(StateError::NotFound(id)))) => {
            eprintln!("container not found: {}", id);
            std::process::exit(1);
        }
        Err(CliError::Shim(ShimError::Config(ConfigError::NotFound(path)))) => {
            eprintln!("config not found: {}", path.display());
            std::process::exit(1);
        }
        Err(_) => {
            println!("{:?}", result);
            panic!("failed to execute command")
        }
    }
}

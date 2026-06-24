use crate::{
    shim::ShimError,
    state::{State, StateError},
};
use clap::{Parser, Subcommand};
use nix::fcntl::{FcntlArg, FdFlag};
use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    path::PathBuf,
    process,
};

mod config;
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
        ready_fd: i32,
    },
}

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
        } => create(container_id, bundle_path).map_err(|e| e.into()),
        Commands::Legacy { bundle_path } => legacy::main(bundle_path).map_err(|e| e.into()),
        Commands::Shim {
            container_id,
            ready_fd,
        } => shim::run(container_id, *ready_fd).map_err(|e| e.into()),
    };
    match result {
        Err(CliError::Create(CreateError::State(StateError::AlreadyExists(id)))) => {
            eprintln!("container already exists: {}", id);
            std::process::exit(1);
        }
        Err(CliError::Create(CreateError::State(StateError::NotFound(id)))) => {
            eprintln!("container not found: {}", id);
            std::process::exit(1);
        }
        _ => panic!("failed to execute command"),
    }
}

enum CreateError {
    State(StateError),
    Io(io::Error),
    Syscall(nix::Error),
}

impl From<StateError> for CreateError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

impl From<io::Error> for CreateError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<nix::Error> for CreateError {
    fn from(value: nix::Error) -> Self {
        Self::Syscall(value)
    }
}

fn create(container_id: &String, bundle_path: &PathBuf) -> Result<(), CreateError> {
    State::new(container_id, bundle_path.to_path_buf(), None)?;
    run_shim(container_id)?;
    Ok(())
}

fn run_shim(id: &str) -> Result<(), CreateError> {
    let (mut recv_shim_ready, send_shim_ready) = std::io::pipe()?;

    nix::fcntl::fcntl(&send_shim_ready, FcntlArg::F_SETFD(FdFlag::empty()))?;

    let program = std::env::current_exe()?;
    process::Command::new(program)
        .arg("__shim")
        .arg(id)
        .arg(send_shim_ready.as_raw_fd().to_string())
        .spawn()?;

    let mut buffer = [0u8; 1];
    recv_shim_ready.read(&mut buffer)?;
    Ok(())
}

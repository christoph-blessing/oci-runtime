use crate::state::State;
use clap::{Parser, Subcommand};
use nix::{
    fcntl::{FcntlArg, FdFlag},
    unistd,
};
use std::{fs, io::Read, os::fd::AsRawFd, path::PathBuf, process};

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

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Create {
            container_id,
            bundle_path,
        } => create(container_id, bundle_path),
        Commands::Legacy { bundle_path } => {
            legacy::main(bundle_path);
        }
        Commands::Shim {
            container_id,
            ready_fd,
        } => {
            shim::run(container_id, *ready_fd);
        }
    }
}

fn create(container_id: &String, bundle_path: &PathBuf) {
    State::new(container_id, bundle_path.to_path_buf(), None).expect("failed to create new state");

    run_shim(container_id);
}

fn run_shim(id: &str) {
    let (mut recv_shim_ready, send_shim_ready) =
        std::io::pipe().expect("failed to create shim ready pipe");

    nix::fcntl::fcntl(&send_shim_ready, FcntlArg::F_SETFD(FdFlag::empty()))
        .expect("failed to remove close-on-exec flag from shim ready pipe");

    let program = std::env::current_exe().expect("failed to get current exe");
    process::Command::new(program)
        .arg("__shim")
        .arg(id)
        .arg(send_shim_ready.as_raw_fd().to_string())
        .spawn()
        .expect("failed to spawn shim");

    let mut buffer = [0u8; 1];
    recv_shim_ready
        .read(&mut buffer)
        .expect("failed to receive shim ready signal");
}

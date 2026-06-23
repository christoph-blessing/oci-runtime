use clap::{Parser, Subcommand};
use nix::unistd;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    os::fd::{AsRawFd, BorrowedFd},
    path::PathBuf,
    process,
};

mod config;
mod legacy;

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
    #[command(hide = true)]
    Shim {
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
        Commands::Shim { ready_fd } => {
            let mut buffer = [0u8; 1];
            nix::unistd::write(unsafe { BorrowedFd::borrow_raw(*ready_fd) }, &mut buffer);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct State {
    oci_version: String,
    id: String,
    status: Status,
    pid: Option<i32>,
    bundle: PathBuf,
    anntotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
enum Status {
    Creating,
    Created,
    Running,
    Stopped,
}

fn create(container_id: &String, bundle_path: &PathBuf) {
    let container_dir = PathBuf::from(format!(
        "/run/user/{}/oci-runtime/{}",
        unistd::getuid(),
        container_id
    ));
    if container_dir.exists() {
        eprintln!("provided id is not unique: {}", container_id);
        process::exit(1);
    }
    fs::create_dir_all(&container_dir).expect("failed to create container directory");

    let state = State {
        oci_version: String::from("1.3.0"),
        id: container_id.to_string(),
        status: Status::Creating,
        pid: None,
        bundle: bundle_path.to_path_buf(),
        anntotations: None,
    };
    let json = serde_json::to_string(&state).expect("failed to serialize state");
    fs::write(container_dir.join("state.json"), json).expect("failed to write state");

    let (mut pipe_reader, pipe_writer) = std::io::pipe().expect("failed to create pipe");
    let program = std::env::current_exe().expect("failed to get current exe");
    let child = process::Command::new(program)
        .arg("shim")
        .arg(pipe_writer.as_raw_fd().to_string())
        .spawn()
        .expect("failed to spawn child");
    let mut buffer = [0u8; 1];
    pipe_reader
        .read(&mut buffer)
        .expect("failed to read from pipe");
}

mod cli;
mod cmd;
mod config;
mod shim;
mod state;

fn main() {
    match cli::run() {
        Ok(_) => std::process::exit(0),
        Err(_) => std::process::exit(1),
    }
}

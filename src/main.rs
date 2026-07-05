mod cli;
mod config;
mod create;
mod kill;
mod shim;
mod start;
mod state;

fn main() {
    match cli::run() {
        Ok(_) => std::process::exit(0),
        Err(_) => std::process::exit(1),
    }
}

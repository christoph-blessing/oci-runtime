mod cli;
mod cmd;
mod config;
mod shim;
mod state;

fn main() {
    match cli::run() {
        Ok(code) => std::process::exit(code),
        Err(_) => std::process::exit(1),
    }
}

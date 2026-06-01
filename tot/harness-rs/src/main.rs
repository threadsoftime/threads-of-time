mod config;
mod auth;
mod registry;
mod audit;

fn main() {
    println!("harness-rs {}", env!("CARGO_PKG_VERSION"));
}

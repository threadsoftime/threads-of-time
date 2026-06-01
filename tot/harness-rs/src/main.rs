mod config;
mod auth;
mod registry;
mod audit;
mod db_client;

fn main() {
    println!("harness-rs {}", env!("CARGO_PKG_VERSION"));
}

mod config;
mod auth;
mod registry;
mod audit;
mod db_client;
mod ac_client;
mod dispatch;
mod mcp;
mod error;
mod rest;
#[cfg(test)] mod test_support;

fn main() {
    println!("harness-rs {}", env!("CARGO_PKG_VERSION"));
}

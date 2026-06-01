mod config;
mod auth;
mod registry;
mod audit;
mod db_client;
mod ac_client;
#[cfg(test)] mod test_support;

fn main() {
    println!("harness-rs {}", env!("CARGO_PKG_VERSION"));
}

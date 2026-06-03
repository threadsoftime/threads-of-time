// brain-rs entrypoint. Modules live in lib.rs so integration tests can use them.
fn main() {
    println!("brain-rs {}", env!("CARGO_PKG_VERSION"));
}

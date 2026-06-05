//! exec-rs binary entry point (stub for M0 — standalone deployment not yet needed).
//!
//! exec-rs is a library-first crate; the binary exists for future standalone
//! deployment. For M0 it does nothing beyond confirming the crate compiles.

fn main() {
    eprintln!("[exec-rs] starting — no HTTP server in M0 stub");
}

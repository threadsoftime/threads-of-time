// build.rs — rerun triggers for compile-time embedded resources.
//
// The migration SQL files and the prompt template are embedded via include_str!
// in src/state.rs and src/app.rs respectively.  Cargo only re-compiles a
// source file when its .rs dependencies change; the include_str! macro does not
// automatically register the embedded file as a dependency.  Declaring them
// here causes Cargo to re-run the build script (and therefore re-compile) when
// the embedded files change.
fn main() {
    // Migrations embedded in src/state.rs
    println!("cargo:rerun-if-changed=migrations/0001_living_bots.sql");
    println!("cargo:rerun-if-changed=migrations/0002_add_last_event_id.sql");
    println!("cargo:rerun-if-changed=migrations/0004_subset_tier.sql");
    // Prompt template embedded in src/app.rs (Phase 4)
    println!("cargo:rerun-if-changed=../brain/prompts/decide_v1.txt");
}

// build.rs — rerun triggers for compile-time embedded resources.
//
// The migration SQL files and the prompt template are embedded via include_str!
// in src/state.rs and src/app.rs respectively.  Cargo only re-compiles a
// source file when its .rs dependencies change; the include_str! macro does not
// automatically register the embedded file as a dependency.  Declaring them
// here causes Cargo to re-run the build script (and therefore re-compile) when
// the embedded files change.
fn main() {
    // Existing: re-run when embedded resources change.
    println!("cargo:rerun-if-changed=migrations/0001_living_bots.sql");
    println!("cargo:rerun-if-changed=migrations/0002_add_last_event_id.sql");
    println!("cargo:rerun-if-changed=migrations/0004_subset_tier.sql");
    println!("cargo:rerun-if-changed=prompts/decide_v1.txt");

    // Build identity. Priority: BRAIN_BUILD_SHA env-arg → git → "unknown".
    println!("cargo:rerun-if-env-changed=BRAIN_BUILD_SHA");
    // Re-run when the resolved git HEAD/refs change. The .git dir lives at the
    // workspace/repo root, not the crate dir, so resolve it via `git rev-parse
    // --absolute-git-dir` (absolute path) rather than a fragile crate-relative path.
    if let Ok(out) = std::process::Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
    {
        if out.status.success() {
            let git_dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !git_dir.is_empty() {
                println!("cargo:rerun-if-changed={git_dir}/HEAD");
                println!("cargo:rerun-if-changed={git_dir}/refs");
            }
        }
    }

    let sha = std::env::var("BRAIN_BUILD_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BRAIN_BUILD_SHA={sha}");

    let build_time = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BRAIN_BUILD_TIME={build_time}");
}

fn main() {
    let amalgam = "vendor/sqlite-vec/sqlite-vec.c";

    // Re-run if the amalgamation source changes.
    println!("cargo:rerun-if-changed={amalgam}");
    println!("cargo:rerun-if-changed=vendor/sqlite-vec/sqlite-vec.h");

    let mut build = cc::Build::new();
    build
        // Compile as C (not C++).
        .file(amalgam)
        // SQLITE_CORE: compiled into the host SQLite (rusqlite bundled), not as
        // a loadable extension. Skips SQLITE_EXTENSION_INIT2 so the function
        // links directly against rusqlite's sqlite3_* symbols.
        .define("SQLITE_CORE", None)
        // SQLITE_VEC_STATIC: suppresses __declspec(dllexport) on Windows; no-op
        // on Linux/macOS (SQLITE_VEC_API is already empty there).
        .define("SQLITE_VEC_STATIC", None)
        // The amalgamation includes sqlite3.h (under SQLITE_CORE); include the
        // vendor dir for the sqlite-vec.h self-reference.
        .include("vendor/sqlite-vec");

    // The amalgamation does `#include "sqlite3.h"` under SQLITE_CORE. Point cc
    // at the *bundled* SQLite header that libsqlite3-sys ships and the static
    // lib is built from, so sqlite-vec compiles against the exact SQLite it
    // links against — not a (possibly absent, possibly mismatched) system one.
    //
    // libsqlite3-sys carries `links = "sqlite3"`; a DIRECT dep edge from this
    // crate (see Cargo.toml) makes Cargo export DEP_SQLITE3_INCLUDE here. The
    // edge through rusqlite alone is transitive and would NOT set it. This is
    // load-bearing for the musl/scratch container build (no system sqlite3.h).
    let inc = std::env::var("DEP_SQLITE3_INCLUDE").expect(
        "DEP_SQLITE3_INCLUDE must be set by libsqlite3-sys (direct dep with \
         `links = \"sqlite3\"`); the vendored sqlite-vec amalgamation needs the \
         bundled sqlite3.h to compile under SQLITE_CORE",
    );
    build.include(inc);

    build.compile("sqlite_vec");
}

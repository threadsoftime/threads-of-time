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

    // musl / Linux cross-compilation fix.
    //
    // Root cause: sqlite-vec.c contained a block (lines 64-74 in the vendored
    // amalgamation, now patched in vendor/sqlite-vec/sqlite-vec.c) that tried
    // to re-typedef uint8_t / uint16_t / uint64_t via BSD aliases:
    //
    //   #ifndef _WIN32 / __EMSCRIPTEN__ / __COSMOPOLITAN__ / __wasi__
    //   typedef u_int8_t uint8_t;   // BSD alias — NOT in musl
    //   typedef u_int16_t uint16_t;
    //   typedef u_int64_t uint64_t;
    //   #endif
    //
    // <stdint.h> is already included above the block and defines the correct
    // types. The block was an unnecessary redeclaration — valid only when the
    // BSD aliases exist AND expand to the identical underlying type. It failed
    // in two distinct ways on Linux:
    //
    //   musl-gcc 1.2.5 (Heimdal, Debian musl-tools): u_int8_t not defined at
    //     all — even with _GNU_SOURCE (musl does not expose BSD aliases via
    //     _GNU_SOURCE; they must come from _BSD_SOURCE, which sqlite-vec does
    //     not define). The compiler falls back to implicit int, making u8 = int
    //     and triggering incompatible-pointer-type errors throughout.
    //
    //   zig-cc 0.16 (local musl cross via cargo-zigbuild): defines uint64_t as
    //     `unsigned _Int64` (a clang internal alias); redefining it as
    //     `unsigned long long` is a type-name conflict.
    //
    // Fix applied: the vendored sqlite-vec.c is patched to guard the block with
    // an additional `#ifndef __linux__`. On Linux (glibc and musl), <stdint.h>
    // provides the correct definitions; the BSD-alias block is skipped. On
    // macOS / FreeBSD the block is also skipped (they have their own <stdint.h>
    // already). No build.rs define needed — the C source patch is self-contained
    // and robust across compilers.
    //
    // Previous attempts that did NOT work:
    //   * -D_GNU_SOURCE (bake #1): musl ignores it for BSD aliases.
    //   * -Du_int8_t="unsigned char" etc. (bake #2 candidate): resolves the
    //     musl-gcc case but still fails zig-cc due to the unsigned _Int64
    //     conflict for uint64_t.

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

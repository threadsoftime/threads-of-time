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

    // musl cross-compilation fix — applies to x86_64-unknown-linux-musl (the
    // Heimdal container target) but NOT to macOS or glibc Linux targets.
    //
    // Root cause: sqlite-vec.c lines 64-74 contain a platform guard that
    // intentionally skips a `typedef u_int8_t uint8_t` block only for _WIN32,
    // __EMSCRIPTEN__, __COSMOPOLITAN__, and __wasi__. On any other target
    // (including Linux/musl) the block runs. musl's standard headers do NOT
    // export `u_int8_t` by default — it is a BSD extension available only when
    // _BSD_SOURCE or _GNU_SOURCE is defined (musl sys/types.h §2). Without it,
    // musl-gcc (in pre-C99 implicit-int mode) treats the undeclared `u_int8_t`
    // as `int`. The typedef chain becomes:
    //   typedef int uint8_t;   // wrong — should be unsigned char
    //   typedef uint8_t u8;    // so u8 = int
    // Functions like bitmap_copy/bitmap_get take `u8 *` (i.e. `int *`) but the
    // call sites pass `unsigned char *` → the pointer-type mismatch the musl-gcc
    // error reports: "u8 * {aka int *}". This IS a real codegen type bug under
    // musl-gcc WITHOUT this fix: the pointer types differ in sign and (on some
    // ABI) alignment assumptions.
    //
    // Fix: define _GNU_SOURCE for the musl target. musl honours _GNU_SOURCE as
    // an alias for _BSD_SOURCE; it causes <sys/types.h> to define u_int8_t as
    // `unsigned char` — the same underlying type that <stdint.h> already gave
    // uint8_t. C11 §6.7.8 permits redeclaring a typedef to the IDENTICAL
    // underlying type, so the repeated `typedef unsigned char uint8_t` is legal.
    // Result: u8 = uint8_t = unsigned char — correct and consistent with the
    // call sites. No miscompilation of bitmap operations; vec0 on-disk format
    // is unchanged (it is determined by the SQLite virtual-table schema, not
    // by this typedef).
    //
    // macOS / glibc Linux: clang + Apple/BSD headers define u_int8_t via
    // <sys/types.h> unconditionally, so the typedef was always correct there.
    // That is why the bug only manifested under musl-gcc.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
        build.define("_GNU_SOURCE", None);
    }

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

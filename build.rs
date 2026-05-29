//! Build script.
//!
//! `intel_tex_2` bundles Intel ISPC (C++) object files for the BC7 encoder.
//! Their C++ runtime symbols (e.g. `__gxx_personality_v0`) must be resolved
//! by libstdc++ at the final link step. On Linux GNU toolchains the default
//! link line can drop libstdc++, so we emit the link requirement here.
//!
//! Emitting it from a build script (rather than only `.cargo/config.toml`,
//! which is local to this repo) means downstream binaries that depend on
//! this crate as a library link libstdc++ automatically — no extra config
//! needed by consumers. Windows (MSVC) and macOS link the C++ runtime
//! through their own mechanisms and don't need this.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("linux") && target.contains("gnu") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}

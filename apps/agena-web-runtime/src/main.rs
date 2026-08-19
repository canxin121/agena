//! Minimal hosted-Web runtime entry point.
//!
//! The full Agena daemon and TUI intentionally depend on OS sockets, process
//! control, PTYs, and terminal events. Emscripten cannot provide those APIs.
//! This executable gives Web targets a real Agena-owned runtime artifact while
//! the static Web UI remains packaged alongside it.

const ABI_VERSION: u32 = 1;

fn main() {
    println!(
        "Agena Web Runtime {} (ABI {})",
        env!("CARGO_PKG_VERSION"),
        ABI_VERSION
    );
}

/// Stable numeric ABI version for JavaScript/native embedding shims.
#[unsafe(no_mangle)]
pub extern "C" fn agena_web_runtime_abi_version() -> u32 {
    ABI_VERSION
}

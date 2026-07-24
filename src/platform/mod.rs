#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    PlatformError, harden_stdio_inheritance, process_identity_for_child, process_identity_matches,
    terminate_if_matches,
};

#[cfg(not(windows))]
compile_error!("maa-mcp-adapter currently supports Windows only");

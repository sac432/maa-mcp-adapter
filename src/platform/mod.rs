#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    PlatformError, assign_new_job, harden_stdio_inheritance, process_identity_for_child,
    process_identity_matches, request_graceful_stop, terminate_if_matches, terminate_job,
    wait_for_exit,
};

#[cfg(not(windows))]
compile_error!("maa-mcp-adapter currently supports Windows only");

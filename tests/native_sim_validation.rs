//! Integration test target for native_sim firmware validation.
//!
//! Top-level wrapper only; implementations live in platform modules.

#[cfg(unix)]
#[path = "native_sim_validation/unix.rs"]
mod unix;

#[cfg(windows)]
#[path = "native_sim_validation/windows.rs"]
mod windows;

mod common;

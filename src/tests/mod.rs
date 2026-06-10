// src/tests/mod.rs
//
// Test submodules.
// Each submodule is gated behind its own `#[cfg(test)]` block
// so the compiler only sees them during `cargo test`.

#[cfg(test)]
mod git_engine_tests;

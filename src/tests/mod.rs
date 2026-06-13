// src/tests/mod.rs
//
// Test submodules.
// Each submodule is gated behind its own `#[cfg(test)]` block
// so the compiler only sees them during `cargo test`.

#[cfg(test)]
mod git_engine_tests;

// Gap 2 — timeline_filter and FilterState unit tests.
#[cfg(test)]
mod timeline_filter_tests;

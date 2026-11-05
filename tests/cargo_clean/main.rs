//! Sutra's clean held against the cargo that runs these tests (`cargo test` names it in
//! `CARGO`): the same workspace and folders as `cargo metadata` reports (`resolution`), the same
//! files deleted as `cargo clean` deletes (`deletion`), and cargo's own builds waiting for the
//! clean or stopping it (`locking`). Every child cargo starts from an empty environment holding
//! only `PATH`, `HOME`, `RUSTUP_TOOLCHAIN`, a `CARGO_HOME` inside the test's own folder, and the
//! variables a test sets on purpose.

mod deletion;
mod fixture;
mod locking;
mod resolution;

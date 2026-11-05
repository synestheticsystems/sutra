//! The static library the SwiftUI app links: Sutra's core, with the scaffolding UniFFI generates
//! for the dashboard. It has nothing of its own; it exists so that only the app's build makes the
//! static library, not every build of the core.

pub use sutra;

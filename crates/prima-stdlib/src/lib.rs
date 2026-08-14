pub mod io;
pub mod linalg;
pub mod num;
pub mod physics;
pub mod plot;
pub mod stats;
pub mod sys;
pub mod time;

/// Register every Rust-hosted stdlib namespace (spec §18) into the runtime registry.
/// Idempotent; call once at startup (the CLI does this in `main`).
pub fn init() {
    linalg::register();
    stats::register();
    io::register();
    physics::register();
    plot::register();
    sys::register();
    time::register();
    num::register();
}

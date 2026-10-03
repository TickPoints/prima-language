//! Prima standard library: embedded `.pra` signature modules bound to Rust-hosted `@builtin`
//! implementations (spec §18.4).
//!
//! The library is split into four opt-in Cargo tiers — `core` (default), `system`, `advanced`,
//! and `render` — so an embedder can pick the surface it needs (and avoid the heavier
//! dependencies). The `full` aggregate is what the `prima` binary ships. See `Cargo.toml`.

#[cfg(feature = "core")]
pub mod collections;
#[cfg(feature = "system")]
pub mod io;
#[cfg(feature = "advanced")]
pub mod linalg;
#[cfg(feature = "advanced")]
pub mod math;
#[cfg(feature = "core")]
mod native_docs;
#[cfg(feature = "core")]
pub mod num;
#[cfg(feature = "advanced")]
pub mod physics;
#[cfg(feature = "advanced")]
pub mod plot;
#[cfg(feature = "advanced")]
pub mod stats;
#[cfg(feature = "core")]
pub mod string;
#[cfg(feature = "system")]
pub mod sys;
#[cfg(feature = "system")]
pub mod time;

/// Register every enabled stdlib implementation and its embedded `.pra` signature module
/// (spec §18.4): `@builtin` declarations in the `.pra` bind to the registered implementations.
///
/// Registration is gated by the Cargo tiers (`core`/`system`/`advanced`/`render`): only the
/// modules of the enabled tiers become importable.
pub fn init() {
    // —— core: built-in classes + `num` ——
    #[cfg(feature = "core")]
    {
        prima_runtime::stdlib::register_module_source("num", include_str!("modules/num.pra"));
        // builtin-class method modules (spec §18.1): the `class` definitions the runtime loads
        // lazily when a builtin value method is called, and that `prima doc --stdlib` lists
        // offline (spec §20).
        prima_runtime::stdlib::register_module_source(
            "core::string",
            include_str!("modules/string.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "core::array",
            include_str!("modules/array.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "core::dict",
            include_str!("modules/dict.pra"),
        );
        prima_runtime::stdlib::register_module_source("core::set", include_str!("modules/set.pra"));
        prima_runtime::stdlib::register_module_source(
            "core::number",
            include_str!("modules/number.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "core::char",
            include_str!("modules/char.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "core::tuple",
            include_str!("modules/tuple.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "core::option",
            include_str!("modules/option.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "core::result",
            include_str!("modules/result.pra"),
        );
        num::register();
        string::register();
        collections::register();
        // doc registry for the builtin classes (spec §4.1/§16.4)
        native_docs::register();
    }

    // —— system: io / time / sys ——
    #[cfg(feature = "system")]
    {
        prima_runtime::stdlib::register_module_source("io", include_str!("modules/io.pra"));
        prima_runtime::stdlib::register_module_source("time", include_str!("modules/time.pra"));
        prima_runtime::stdlib::register_module_source(
            "sys::path",
            include_str!("modules/sys_path.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "sys::env",
            include_str!("modules/sys_env.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "sys::os",
            include_str!("modules/sys_os.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "sys::process",
            include_str!("modules/sys_process.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "sys::fs",
            include_str!("modules/sys_fs.pra"),
        );
        prima_runtime::stdlib::register_module_source(
            "sys::term",
            include_str!("modules/sys_term.pra"),
        );
        io::register();
        sys::register();
        time::register();
    }

    // —— advanced: linalg / stats / physics / plot / math ——
    #[cfg(feature = "advanced")]
    {
        prima_runtime::stdlib::register_module_source("linalg", include_str!("modules/linalg.pra"));
        prima_runtime::stdlib::register_module_source("stats", include_str!("modules/stats.pra"));
        prima_runtime::stdlib::register_module_source("plot", include_str!("modules/plot.pra"));
        prima_runtime::stdlib::register_module_source("math", include_str!("modules/math.pra"));
        linalg::register();
        stats::register();
        plot::register();
        math::register();
        // pure-data namespaces
        physics::register();
    }
}

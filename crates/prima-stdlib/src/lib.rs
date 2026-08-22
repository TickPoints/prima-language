pub mod io;
pub mod linalg;
pub mod num;
pub mod physics;
pub mod plot;
pub mod stats;
mod string_docs;
pub mod sys;
pub mod time;

/// Register every Rust-hosted stdlib implementation and its embedded `.pra` signature module
/// (spec §18.4): `@builtin` declarations in the `.pra` bind to the registered implementations.
pub fn init() {
    // embedded signature sources
    prima_runtime::stdlib::register_module_source("linalg", include_str!("modules/linalg.pra"));
    prima_runtime::stdlib::register_module_source("stats", include_str!("modules/stats.pra"));
    prima_runtime::stdlib::register_module_source("io", include_str!("modules/io.pra"));
    prima_runtime::stdlib::register_module_source("plot", include_str!("modules/plot.pra"));
    prima_runtime::stdlib::register_module_source("sys::path", include_str!("modules/sys_path.pra"));
    prima_runtime::stdlib::register_module_source("sys::env", include_str!("modules/sys_env.pra"));
    prima_runtime::stdlib::register_module_source("sys::os", include_str!("modules/sys_os.pra"));
    prima_runtime::stdlib::register_module_source("time", include_str!("modules/time.pra"));
    prima_runtime::stdlib::register_module_source("num", include_str!("modules/num.pra"));
    // `String` is a native runtime class, not a stdlib module, but registering its signature
    // module under `core::string` lets `prima doc --stdlib` list the class offline (spec §20).
    prima_runtime::stdlib::register_module_source("core::string", include_str!("modules/string.pra"));
    // implementations
    linalg::register();
    stats::register();
    io::register();
    plot::register();
    sys::register();
    time::register();
    num::register();
    // pure-data namespaces
    physics::register();
    // doc registry for the native `String` class (spec §4.1/§16.4)
    string_docs::register();
}

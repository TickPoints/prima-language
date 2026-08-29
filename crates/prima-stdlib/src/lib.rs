pub mod collections;
pub mod io;
pub mod linalg;
mod native_docs;
pub mod num;
pub mod physics;
pub mod plot;
pub mod stats;
pub mod string;
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
    prima_runtime::stdlib::register_module_source(
        "sys::path",
        include_str!("modules/sys_path.pra"),
    );
    prima_runtime::stdlib::register_module_source("sys::env", include_str!("modules/sys_env.pra"));
    prima_runtime::stdlib::register_module_source("sys::os", include_str!("modules/sys_os.pra"));
    prima_runtime::stdlib::register_module_source("time", include_str!("modules/time.pra"));
    prima_runtime::stdlib::register_module_source("num", include_str!("modules/num.pra"));
    // builtin-class method modules (spec §18.1): the `class` definitions the runtime loads lazily
    // when a builtin value method is called, and that `prima doc --stdlib` lists offline (spec §20).
    prima_runtime::stdlib::register_module_source(
        "core::string",
        include_str!("modules/string.pra"),
    );
    prima_runtime::stdlib::register_module_source("core::array", include_str!("modules/array.pra"));
    prima_runtime::stdlib::register_module_source("core::dict", include_str!("modules/dict.pra"));
    prima_runtime::stdlib::register_module_source("core::set", include_str!("modules/set.pra"));
    prima_runtime::stdlib::register_module_source(
        "core::number",
        include_str!("modules/number.pra"),
    );
    prima_runtime::stdlib::register_module_source("core::char", include_str!("modules/char.pra"));
    prima_runtime::stdlib::register_module_source("core::tuple", include_str!("modules/tuple.pra"));
    prima_runtime::stdlib::register_module_source(
        "core::option",
        include_str!("modules/option.pra"),
    );
    prima_runtime::stdlib::register_module_source(
        "core::result",
        include_str!("modules/result.pra"),
    );
    // implementations
    linalg::register();
    stats::register();
    io::register();
    plot::register();
    sys::register();
    time::register();
    num::register();
    string::register();
    collections::register();
    // pure-data namespaces
    physics::register();
    // doc registry for the builtin classes (spec §4.1/§16.4)
    native_docs::register();
}

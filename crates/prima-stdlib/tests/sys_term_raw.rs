#![cfg(feature = "system")]

use prima_core::Value;
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

#[test]
fn raw_mode_worker() {
    let mode = match std::env::var("PRIMA_RAW_TEST") {
        Ok(mode) => mode,
        Err(_) => return,
    };
    assert!(matches!(
        eval("import sys::term; sys::term::is_raw()"),
        Value::Bool(false)
    ));
    assert!(matches!(
        eval("import sys::term; sys::term::raw_mode(false)"),
        Value::Result(Ok(_))
    ));
    let result = eval("import sys::term; sys::term::raw_mode(true)");
    if mode == "tty" {
        assert!(matches!(result, Value::Result(Ok(_))), "{result:?}");
        assert!(matches!(
            eval("import sys::term; sys::term::is_raw()"),
            Value::Bool(true)
        ));
        assert!(matches!(
            eval("import sys::term; sys::term::raw_mode(true)"),
            Value::Result(Ok(_))
        ));
        assert!(matches!(
            eval("import sys::term; sys::term::raw_mode(false)"),
            Value::Result(Ok(_))
        ));
        assert!(matches!(
            eval("import sys::term; sys::term::raw_mode(false)"),
            Value::Result(Ok(_))
        ));
    } else {
        assert!(matches!(result, Value::Result(Err(_))), "{result:?}");
    }
    assert!(matches!(
        eval("import sys::term; sys::term::is_raw()"),
        Value::Bool(false)
    ));
}

#[cfg(unix)]
#[test]
fn raw_mode_pty_and_non_tty() {
    let script = r#"
import os, pty, subprocess, sys, termios
exe = sys.argv[1]
for mode in ('tty', 'non-tty'):
    master, slave = pty.openpty() if mode == 'tty' else (None, os.open(os.devnull, os.O_RDONLY))
    before = termios.tcgetattr(slave) if mode == 'tty' else None
    env = dict(os.environ, PRIMA_RAW_TEST=mode)
    result = subprocess.run([exe, '--exact', 'raw_mode_worker', '--nocapture'], stdin=slave,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env,
                            start_new_session=True)
    if mode == 'tty':
        assert termios.tcgetattr(slave) == before, 'terminal attributes were not restored'
        os.close(master)
    os.close(slave)
    if result.returncode:
        sys.stdout.buffer.write(result.stdout)
        sys.exit(result.returncode)
"#;
    let status = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(std::env::current_exe().unwrap())
        .status()
        .expect("python3 is required for pseudo-terminal tests");
    assert!(status.success());
}

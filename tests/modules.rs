use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use prima_runtime::Evaluator;

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "prima_mod_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &TempDir, main: &str) -> Result<String, prima_runtime::RuntimeError> {
    let out = Rc::new(RefCell::new(String::new()));
    let out_c = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| out_c.borrow_mut().push_str(&s));
    let root = dir.write("main.pra", main);
    ev.eval_file(&root)?;
    Ok(out.borrow().clone())
}

#[test]
fn import_namespace_access() {
    let dir = TempDir::new();
    dir.write(
        "mymath.pra",
        "pub let square(x) = x^2;\nlet helper(x) = x + 1",
    );
    assert_eq!(
        run(&dir, "import mymath;\nprintln(mymath::square(3))").unwrap(),
        "9\n"
    );
}

#[test]
fn private_item_not_exported() {
    let dir = TempDir::new();
    dir.write(
        "mymath.pra",
        "pub let square(x) = x^2;\nlet helper(x) = x + 1",
    );
    assert!(run(&dir, "import mymath;\nprintln(mymath::helper(3))").is_err());
}

#[test]
fn import_with_alias() {
    let dir = TempDir::new();
    dir.write("mymath.pra", "pub let square(x) = x^2");
    assert_eq!(
        run(&dir, "import mymath as m;\nprintln(m::square(4))").unwrap(),
        "16\n"
    );
}

#[test]
fn from_import_with_alias() {
    let dir = TempDir::new();
    dir.write("mymath.pra", "pub let square(x) = x^2");
    assert_eq!(
        run(&dir, "from mymath import square as sq;\nprintln(sq(4))").unwrap(),
        "16\n"
    );
}

#[test]
fn from_import_star() {
    let dir = TempDir::new();
    dir.write(
        "mymath.pra",
        "pub let square(x) = x^2;\npub const K: Integer = 3",
    );
    assert_eq!(
        run(
            &dir,
            "from mymath import *;\nprintln(square(2));\nprintln(K)"
        )
        .unwrap(),
        "4\n3\n"
    );
}

#[test]
fn conflicting_imports_rejected() {
    let dir = TempDir::new();
    dir.write("a.pra", "pub let square(x) = x^2");
    dir.write("b.pra", "pub let square(x) = x + 100");
    assert!(run(&dir, "from a import square;\nfrom b import square").is_err());
}

#[test]
fn polluting_config_rejected_in_module() {
    let dir = TempDir::new();
    dir.write("bad.pra", "config { domain := real }\npub let f(x) = x");
    assert!(run(&dir, "import bad;\nprintln(1)").is_err());
}

#[test]
fn nested_module_path() {
    let dir = TempDir::new();
    std::fs::create_dir_all(dir.0.join("linalg")).unwrap();
    dir.write("linalg/fft.pra", "pub let double(x) = x * 2");
    assert_eq!(
        run(
            &dir,
            "import linalg::fft;\nprintln(linalg::fft::double(21))"
        )
        .unwrap(),
        "42\n"
    );
}

#[test]
fn module_config_fraction_applies() {
    let dir = TempDir::new();
    dir.write(
        "frac.pra",
        "config { fraction := false }\npub let third = 1/3",
    );
    assert_eq!(
        run(&dir, "from frac import third;\nprintln(third)").unwrap(),
        "0.3333333333333333\n"
    );
}

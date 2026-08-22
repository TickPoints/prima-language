// Class model tests (spec §4.5, §12.3): definition, associated functions, methods, struct literals,
// visibility, method chaining, and classes exported from modules.
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_fmt(src: &str) -> String {
    let mut ev = Evaluator::new();
    let v = ev.eval_value(src).expect("eval failed");
    ev.format_value(&v)
}

fn run_file(path: &Path) -> String {
    let out = Rc::new(RefCell::new(String::new()));
    let out_c = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| out_c.borrow_mut().push_str(&s));
    ev.eval_file(path).expect("eval failed");
    out.borrow().clone()
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!("prima_class_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
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

#[test]
fn associated_function_and_method_return_field() {
    let src = "class Vec2 {\n    x: F64,\n    y: F64,\n    pub fn new(x, y) -> Self { Vec2 { x, y } }\n    pub fn sum(self) -> F64 { self.x + self.y }\n}\nlet v = Vec2::new(1, 2);\nv.sum()";
    assert_eq!(eval_fmt(src), "3");
}

#[test]
fn private_field_not_accessible_outside() {
    let src = "class C {\n    secret: Integer,\n    pub fn new(s) -> Self { C { secret: s } }\n}\nlet c = C::new(1);\nc.secret";
    assert!(Evaluator::new().eval_value(src).is_err());
}

#[test]
fn pub_field_accessible_and_mutating_method() {
    let src = "class Counter {\n    pub count: Integer,\n    pub fn new(start: Integer) -> Self { Counter { count: start } }\n    pub fn increment(self) -> Self { Counter { count: self.count + 1 } }\n    pub fn value(self) -> Integer { self.count }\n}\nlet c = Counter::new(10);\nlet d = c.increment();\nd.value()";
    assert_eq!(eval(src), Value::Number(Number::from(11)));
    assert_eq!(eval("class Counter {\n    pub count: Integer,\n    pub fn new(start: Integer) -> Self { Counter { count: start } }\n}\nlet c = Counter::new(10);\nc.count"), Value::Number(Number::from(10)));
}

#[test]
fn struct_literal_and_field_access() {
    let src = "class P { pub x: Integer, pub y: Integer }\nlet p = P { x: 1, y: 2 };\np.x";
    assert_eq!(eval(src), Value::Number(Number::from(1)));
}

#[test]
fn struct_literal_update_syntax() {
    let src = "class P { pub x: Integer, pub y: Integer }\nlet a = P { x: 1, y: 2 };\nlet b = P { x: 9, ..a };\nb.y";
    assert_eq!(eval(src), Value::Number(Number::from(2)));
}

#[test]
fn method_chaining_replaces_pipeline() {
    let src = "class Accumulator {\n    pub total: Integer,\n    pub fn new() -> Self { Accumulator { total: 0 } }\n    pub fn add(self, n: Integer) -> Self { Accumulator { total: self.total + n } }\n}\nlet s = Accumulator::new().add(1).add(2).add(3);\ns.total";
    assert_eq!(eval(src), Value::Number(Number::from(6)));
}

#[test]
fn pub_mod_visible_within_module() {
    // `pub(mod)` (spec §15.2): visible inside the defining module — readable from a method of the same class.
    let src = "class Thing {\n    pub(mod) x: Integer,\n    pub fn new(x) -> Self { Thing { x: x } }\n    pub fn get(self) -> Integer { self.x }\n}\nlet t = Thing::new(7);\nt.get()";
    assert_eq!(eval(src), Value::Number(Number::from(7)));
}

#[test]
fn class_in_module_imported_and_called() {
    let dir = TempDir::new();
    dir.write("shapes.pra", "pub class Point {\n    pub x: Integer,\n    pub y: Integer,\n    pub fn new(x, y) -> Self { Point { x, y } }\n    pub fn sum(self) -> Integer { self.x + self.y }\n}\n");
    let main = dir.write("main.pra", "import shapes;\nlet p = shapes::Point::new(3, 4);\nprintln(p.sum());\nprintln(p.x);");
    let out = run_file(&main);
    assert_eq!(out, "7\n3\n");
}

#[test]
fn class_in_module_method_chain() {
    let dir = TempDir::new();
    dir.write("acc.pra", "pub class Acc {\n    pub total: Integer,\n    pub fn new() -> Self { Acc { total: 0 } }\n    pub fn add(self, n: Integer) -> Self { Acc { total: self.total + n } }\n}\n");
    let main = dir.write("main.pra", "import acc;\nlet s = acc::Acc::new().add(5).add(6);\nprintln(s.total);");
    assert_eq!(run_file(&main), "11\n");
}

#[test]
fn method_receives_shallow_copy_of_receiver() {
    let src = "class C {\n    pub x: Integer,\n    pub fn new(x) -> Self { C { x: x } }\n    pub fn poke(self) -> Integer { self.x }\n}\nlet c = C::new(3);\nlet r = c.poke();\nc.x";
    assert_eq!(eval(src), Value::Number(Number::from(3)));
}

#[test]
fn missing_constructor_field_errors() {
    let src = "class P { pub x: Integer, pub y: Integer }\nP { x: 1 }";
    assert!(Evaluator::new().eval_value(src).is_err());
}

#[test]
fn unknown_method_errors() {
    let src = "class C {\n    pub x: Integer,\n    pub fn new(x) -> Self { C { x: x } }\n}\nlet c = C::new(1);\nc.bogus()";
    assert!(Evaluator::new().eval_value(src).is_err());
}

#[test]
fn class_value_formats_as_instance() {
    let mut ev = Evaluator::new();
    let v = ev.eval_value("class V { pub x: F64 }\nV { x: 1.0 }").expect("eval failed");
    assert_eq!(ev.format_value(&v), "class V");
}

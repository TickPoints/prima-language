//! Array value semantics with the shared copy-on-write representation (spec §11.3): cloning an
//! array shares its buffer, and any mutation (`A[i] = v`, the mutating `Array` methods) must be
//! invisible to aliases (`let b = a; b[0] = 9` leaves `a` unchanged).

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_err(src: &str) -> RuntimeError {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .expect_err("expected an error")
}

use prima_runtime::error::RuntimeError;

fn int(n: i64) -> Value {
    Value::Number(Number::from(n))
}

#[test]
fn index_assignment_is_invisible_to_aliases() {
    // `let b = a; b[0] = 9` must not change `a` (spec §11.3 value semantics).
    assert_eq!(eval("let a = [1, 2];\nlet b = a;\nb[0] = 9;\na[0]"), int(1));
    assert_eq!(eval("let a = [1, 2];\nlet b = a;\nb[0] = 9;\nb[0]"), int(9));
}

#[test]
fn push_is_invisible_to_aliases() {
    assert_eq!(eval("let a = [1];\nlet b = a;\nb.push(2);\nlen(a)"), int(1));
    assert_eq!(eval("let a = [1];\nlet b = a;\nb.push(2);\nlen(b)"), int(2));
    assert_eq!(
        eval("let a = [1];\nlet b = a;\nb.append(3);\nb.extend([4, 5]);\nb.insert(0, 0);\nlen(a)"),
        int(1)
    );
}

#[test]
fn pop_remove_clear_are_invisible_to_aliases() {
    assert_eq!(
        eval("let a = [1, 2, 3];\nlet b = a;\nb.pop();\nlen(a)"),
        int(3)
    );
    assert_eq!(
        eval("let a = [1, 2, 3];\nlet b = a;\nb.remove(0);\nlen(a)"),
        int(3)
    );
    assert_eq!(
        eval("let a = [1, 2, 3];\nlet b = a;\nb.clear();\nlen(a)"),
        int(3)
    );
    // The mutating methods still mutate their own binding.
    assert_eq!(
        eval("let a = [1, 2, 3];\nlet b = a;\nb.remove(0);\nb[0]"),
        int(2)
    );
}

#[test]
fn sort_and_reverse_are_invisible_to_aliases() {
    assert_eq!(
        eval("let a = [3, 1, 2];\nlet b = a;\nb.sort();\na"),
        Value::Array(vec![int(3), int(1), int(2)].into())
    );
    assert_eq!(
        eval("let a = [3, 1, 2];\nlet b = a;\nb.reverse();\nb"),
        Value::Array(vec![int(2), int(1), int(3)].into())
    );
}

#[test]
fn compound_index_assignment_reads_old_element() {
    assert_eq!(eval("let a = [1, 2, 3];\na[1] += 10;\na[1]"), int(12));
    assert_eq!(eval("let a = [1, 2, 3];\na[1] -= 2;\na[1]"), int(0));
    // Aliases still do not see the compound update.
    assert_eq!(
        eval("let a = [1, 2];\nlet b = a;\nb[0] += 5;\na[0]"),
        int(1)
    );
}

#[test]
fn mutation_inside_function_does_not_leak_to_caller() {
    // A parameter binding shares the caller's buffer; the mutation must copy-on-write.
    assert_eq!(
        eval("fn f(v) { v.push(9); return len(v); }\nlet a = [1];\nlet r = f(a);\nr + len(a)"),
        int(3)
    );
}

#[test]
fn array_stored_into_itself_is_a_snapshot() {
    // `a[0] = a` stores a snapshot copy, not a self-reference (no cycle, spec §11.3).
    assert_eq!(eval("let a = [1];\na[0] = a;\nlen(a[0])"), int(1));
    assert_eq!(eval("let a = [1];\na.push(a);\nlen(a[1])"), int(1));
    assert_eq!(eval("let a = [1];\nlet b = a;\nb[0] = a;\nlen(a)"), int(1));
}

#[test]
fn array_stored_into_dict_stays_value_semantic() {
    // Storing an array into a dict shares the buffer, so a later mutation of `a` must CoW and
    // leave the dict's copy untouched (same observable behavior as the whole-value copy).
    assert_eq!(
        eval("let a = [1];\nlet d = {};\nd[\"k\"] = a;\na[0] = 9;\nd[\"k\"][0]"),
        int(1)
    );
}

#[test]
fn local_array_mutation_inside_class_method_is_visible() {
    // A method-local array binding is mutated in place across loop iterations (spec §11.3).
    assert_eq!(
        eval(
            "class Sieve {\n    pub fn primes(self, n) -> Integer {\n        let mut mark = [];\n        for i in 0..n { mark.push(true); }\n        mark[0] = false;\n        mark[1] = false;\n        for i in 2..n {\n            let mut j = i * i;\n            while j < n { mark[j] = false; j += i; }\n        }\n        let mut c = 0;\n        for i in 0..n { if mark[i] { c += 1; } }\n        return c;\n    }\n}\nlet s = Sieve {};\ns.primes(50)"
        ),
        int(15)
    );
}

#[test]
fn slice_assignment_is_value_semantic() {
    assert_eq!(
        eval("let a = [1, 2, 3, 4];\nlet b = a;\na[1..3] = [9, 9];\nb"),
        Value::Array(vec![int(1), int(2), int(3), int(4)].into())
    );
    assert_eq!(
        eval("let a = [1, 2, 3, 4];\na[1..3] = [9, 9];\na"),
        Value::Array(vec![int(1), int(9), int(9), int(4)].into())
    );
}

#[test]
fn mutation_error_messages_are_unchanged() {
    let e = eval_err("let x = 5;\nx.push(1)");
    assert!(
        e.to_string().contains("unknown `Number` method `push`"),
        "unexpected error: {e}"
    );
    let e = eval_err("[1, 2].push(1)");
    assert!(
        e.to_string().contains("cannot mutate a temporary value"),
        "unexpected error: {e}"
    );
    let e = eval_err("let a = [1];\na[5] = 0");
    assert!(
        e.to_string().contains("index out of bounds"),
        "unexpected error: {e}"
    );
}

#[test]
fn nested_array_assignment_is_value_semantic() {
    // A nested array shares the outer buffer's element handles; mutating the outer through one
    // alias must not affect the other.
    assert_eq!(
        eval("let a = [[1], [2]];\nlet b = a;\nb[0] = [9];\na[0][0]"),
        int(1)
    );
}

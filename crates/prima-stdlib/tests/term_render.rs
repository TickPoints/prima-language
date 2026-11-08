use std::{cell::RefCell, rc::Rc};

use prima_runtime::Evaluator;

fn output(format: &str, tty: bool) -> String {
    prima_stdlib::init();
    let output = Rc::new(RefCell::new(String::new()));
    let sink = output.clone();
    let mut evaluator =
        Evaluator::with_terminal_sink(move |text| sink.borrow_mut().push_str(&text), tty);
    evaluator
        .eval_src(&format!(
            "config {{ print_format := {format} }} println(x^2);"
        ))
        .unwrap();
    output.borrow().clone()
}

#[test]
fn non_terminal_obeys_print_format() {
    assert_eq!(output("latex", false), "x^{2}\n");
    assert_eq!(output("unicode", false), "x^(2)\n");
    assert_eq!(output("ascii", false), "x^(2)\n");
}

#[test]
fn terminal_obeys_format_and_feature() {
    assert_eq!(output("latex", true), "x^{2}\n");
    assert_eq!(output("ascii", true), "x^(2)\n");
    #[cfg(feature = "term-render")]
    assert_eq!(output("unicode", true), "x²\n");
    #[cfg(not(feature = "term-render"))]
    assert_eq!(output("unicode", true), "x^(2)\n");
}

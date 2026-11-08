use std::{cell::RefCell, rc::Rc};

use prima_runtime::{Evaluator, config::PrintFormat, set_term_formula_renderer};

fn renderer(_: &str, format: PrintFormat) -> Option<String> {
    match format {
        PrintFormat::Unicode => Some("FORMULA".into()),
        _ => None,
    }
}

fn run(src: &str, tty: bool) -> String {
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    let mut ev = Evaluator::with_terminal_sink(move |s| sink.borrow_mut().push_str(&s), tty);
    ev.eval_src(src).unwrap();
    out.borrow().clone()
}

#[test]
fn formats_and_terminal_fallback() {
    set_term_formula_renderer(renderer).unwrap();
    let src = "config { print_format := unicode } println(x^2); println(f\"{x^2}\"); println([x^2, 1/2]);";
    let plain = run(src, false);
    assert!(plain.contains("x^(2)"), "{plain}");
    assert!(plain.contains("(1)/(2)"), "{plain}");
    let tty = run(src, true);
    assert!(tty.starts_with("FORMULA\n"), "{tty}");
    assert!(tty.contains("x^(2)"), "{tty}");
    let ascii = run("config { print_format := ascii } println(x^2);", true);
    assert_eq!(ascii, "x^(2)\n");
    let latex = run("println(x^2);", true);
    assert_eq!(latex, "x^{2}\n");
}

#[test]
fn string_helpers_follow_print_format() {
    for format in ["unicode", "ascii", "latex"] {
        let src = format!(
            "config {{ print_format := {format} }} let v = (x^2, 1/2); println(v); println(f\"{{v}}\"); println(to_string(v)); println(concat(v, v));"
        );
        let output = run(&src, false);
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 4, "{output}");
        assert_eq!(lines[0], lines[1], "{format}");
        assert_eq!(lines[0], lines[2], "{format}");
        assert_eq!(lines[3], format!("{}{}", lines[0], lines[0]), "{format}");
    }
}

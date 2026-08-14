use std::fs;
use std::path::PathBuf;

use prima_runtime::Evaluator;

/// Evaluate an in-memory program that imports the `plot` stdlib namespace (spec §18 / appendix B.4).
/// `eval_value` (not `eval_src`) so that Rust-hosted `import` resolves without a file.
fn run(src: &str) -> bool {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).is_ok()
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn remove(path: &PathBuf) {
    let _ = fs::remove_file(path);
}

#[test]
fn plot_savefig_writes_svg() {
    let path = tmp("prima_plot_line_test.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::title(\"T\");\n\
         plot::xlabel(\"x\");\n\
         plot::plot([0.0, 1.0], [0.0, 1.0], \"line\");\n\
         plot::savefig(\"{}\");",
        path.display()
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.starts_with("<svg"), "content: {content}");
    assert!(content.contains("<polyline"), "content: {content}");
    assert!(content.contains("line"), "series label should be rendered: {content}");
    remove(&path);
}

#[test]
fn plot_clear_then_plot_again() {
    let p1 = tmp("prima_plot_clear1.svg");
    let p2 = tmp("prima_plot_clear2.svg");
    remove(&p1);
    remove(&p2);
    let src = format!(
        "import plot;\n\
         plot::plot([0.0, 1.0], [0.0, 1.0]);\n\
         plot::savefig(\"{}\");\n\
         plot::clear();\n\
         plot::plot([0.0, 2.0], [0.0, 4.0]);\n\
         plot::savefig(\"{}\");",
        p1.display(),
        p2.display()
    );
    assert!(run(&src), "program failed");
    assert!(p1.exists(), "first figure missing");
    assert!(p2.exists(), "figure after clear() missing");
    remove(&p1);
    remove(&p2);
}

#[test]
fn plot_savefig_rejects_non_svg_extension() {
    let path = tmp("prima_plot_not_svg.png");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::plot([0.0, 1.0], [0.0, 1.0]);\n\
         plot::savefig(\"{}\");",
        path.display()
    );
    assert!(!run(&src), "png extension must be rejected");
    assert!(!path.exists(), "no file should be written for a rejected format");
}

#[test]
fn plot_savefig_rejects_non_svg_format() {
    let path = tmp("prima_plot_not_svg2.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::plot([0.0, 1.0], [0.0, 1.0]);\n\
         plot::savefig(\"{}\", \"png\");",
        path.display()
    );
    assert!(!run(&src), "format png must be rejected");
    assert!(!path.exists(), "no file should be written for a rejected format");
}

#[test]
fn plot_scatter_and_bar_render() {
    let path = tmp("prima_plot_scatter_bar.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::scatter([0.0, 1.0, 2.0], [0.0, 1.0, 0.5], \"pts\");\n\
         plot::bar([0.0, 1.0, 2.0], [1.0, 2.0, 3.0]);\n\
         plot::grid(true);\n\
         plot::savefig(\"{}\");",
        path.display()
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.contains("<circle"), "content: {content}");
    assert!(content.contains("<rect"), "content: {content}");
    remove(&path);
}

#[test]
fn plot_show_prints_svg() {
    assert!(run("import plot;\nplot::plot([0.0, 1.0], [0.0, 1.0]);\nplot::show();"));
}

#![cfg(feature = "advanced")]
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use prima_runtime::Evaluator;

/// The `plot` module keeps its accumulated figure in process-global state (spec §18.6), so tests
/// must not run concurrently: otherwise one test's `savefig` observes another test's series and
/// overlay label. Each test binary is a single process, so a local lock is sufficient.
static PLOT_STATE_LOCK: Mutex<()> = Mutex::new(());

fn lock_plot() -> MutexGuard<'static, ()> {
    PLOT_STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Evaluate an in-memory program that imports the `plot` stdlib namespace (spec §18 / appendix B.4).
/// `eval_value` (not `eval_src`) so that Rust-hosted `import` resolves without a file.
///
/// The lock is held for the whole evaluation, i.e. including the `savefig` render, so no other
/// plot test can interleave its own calls.
fn run(src: &str) -> bool {
    let _guard = lock_plot();
    prima_stdlib::init();
    Evaluator::new().eval_value(src).is_ok()
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// Escape a filesystem path for embedding in a Prima string literal (spec §18.1):
/// backslashes in Windows paths must be doubled, and quotes escaped.
fn primed_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
        primed_str(&path.to_string_lossy())
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.starts_with("<svg"), "content: {content}");
    assert!(content.contains("<polyline"), "content: {content}");
    assert!(
        content.contains("line"),
        "series label should be rendered: {content}"
    );
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
        primed_str(&p1.to_string_lossy()),
        primed_str(&p2.to_string_lossy())
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
        primed_str(&path.to_string_lossy())
    );
    assert!(!run(&src), "png extension must be rejected");
    assert!(
        !path.exists(),
        "no file should be written for a rejected format"
    );
}

#[test]
fn plot_savefig_rejects_non_svg_format() {
    let path = tmp("prima_plot_not_svg2.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::plot([0.0, 1.0], [0.0, 1.0]);\n\
         plot::savefig(\"{}\", \"png\");",
        primed_str(&path.to_string_lossy())
    );
    assert!(!run(&src), "format png must be rejected");
    assert!(
        !path.exists(),
        "no file should be written for a rejected format"
    );
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
        primed_str(&path.to_string_lossy())
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.contains("<circle"), "content: {content}");
    assert!(content.contains("<rect"), "content: {content}");
    remove(&path);
}

#[test]
fn plot_show_prints_svg() {
    assert!(run(
        "import plot;\nplot::plot([0.0, 1.0], [0.0, 1.0]);\nplot::show();"
    ));
}

#[test]
fn plot_heatmap_renders_cells_and_colorbar() {
    let path = tmp("prima_plot_heatmap.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::heatmap([[0.0, 1.0], [0.0, 1.0]], \"grid\");\n\
         plot::savefig(\"{}\");",
        primed_str(&path.to_string_lossy())
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.starts_with("<svg"), "content: {content}");
    // The color map spans the low and high control points; the label comes from the colorbar.
    assert!(content.contains("#440154"), "low color missing: {content}");
    assert!(content.contains("#fde725"), "high color missing: {content}");
    assert!(
        content.contains("grid"),
        "overlay label should be rendered: {content}"
    );
    remove(&path);
}

#[test]
fn plot_contour_renders_iso_lines() {
    let path = tmp("prima_plot_contour.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::contour([[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]], 2, \"iso\");\n\
         plot::savefig(\"{}\");",
        primed_str(&path.to_string_lossy())
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.starts_with("<svg"), "content: {content}");
    assert!(content.contains("<polyline"), "content: {content}");
    assert!(
        content.contains("iso"),
        "overlay label should be rendered: {content}"
    );
    remove(&path);
}

#[test]
fn plot_hist_renders_bars() {
    let path = tmp("prima_plot_hist.svg");
    remove(&path);
    let src = format!(
        "import plot;\n\
         plot::hist([1.0, 1.0, 2.0, 2.0, 2.0, 3.0], 3, \"samples\");\n\
         plot::savefig(\"{}\");",
        primed_str(&path.to_string_lossy())
    );
    assert!(run(&src), "program failed");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.starts_with("<svg"), "content: {content}");
    assert!(content.contains("<rect"), "content: {content}");
    remove(&path);
}

#[test]
fn plot_grid_handles_non_finite_values() {
    let path = tmp("prima_plot_nonfinite.svg");
    remove(&path);
    let src = format!(
        "config {{ fraction := false }}\n\
         import plot;\n\
         plot::heatmap([[1.0, 0.0/0.0], [1.0/0.0, 2.0]], \"g\");\n\
         plot::savefig(\"{}\");",
        primed_str(&path.to_string_lossy())
    );
    assert!(run(&src), "non-finite cells must not panic");
    let content = fs::read_to_string(&path).expect("svg file should exist");
    assert!(content.starts_with("<svg"), "content: {content}");
    remove(&path);
}

#[test]
fn plot_heatmap_rejects_non_rectangular_grid() {
    assert!(!run(
        "import plot;\nplot::heatmap([[1.0, 2.0], [3.0]], \"x\");"
    ));
}

#[test]
fn plot_heatmap_rejects_empty_grid() {
    assert!(!run("import plot;\nplot::heatmap([], \"x\");"));
}

#[test]
fn plot_contour_rejects_nonpositive_levels() {
    assert!(!run(
        "import plot;\nplot::contour([[1.0, 2.0], [3.0, 4.0]], 0, \"x\");"
    ));
}

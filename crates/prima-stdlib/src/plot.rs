//! `plot` module (spec §18 / appendix B.4): SVG charting MVP.
//!
//! Plotting keeps *state* between calls: series are accumulated and layout options set until
//! `savefig` renders the whole figure at once. State is process-global (`OnceLock<Mutex<PlotState>>`)
//! because the interpreter calls the `plot` functions sequentially; the module never runs inside rayon.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use prima_core::Value;
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError};

/// Plot series kind (spec §B.4): `plot`/`line` draw lines, `scatter` draws point markers,
/// `bar` draws per-x rectangles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Line,
    Scatter,
    Bar,
}

/// Scalar-grid overlay kind (spec §18.6): `heatmap` renders a colored cell per value, `contour`
/// marches evenly spaced iso-lines across the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridKind {
    Heatmap,
    Contour,
}

/// One accumulated series.
#[derive(Debug, Clone)]
struct Series {
    kind: Kind,
    x: Vec<f64>,
    y: Vec<f64>,
    label: Option<String>,
    color: Option<String>,
    // Stored for future marker styles; the MVP always renders circle markers (spec §B.4).
    #[allow(dead_code)]
    marker: Option<String>,
    linestyle: Option<String>,
}

/// One accumulated scalar-grid overlay (spec §18.6). `z[r][c]` is row `r`, column `c`; row 0 is
/// drawn at the top of the axes and the grid spans `[0, ncols]` x `[0, nrows]` in axis coordinates.
/// The grid is validated to be non-empty and rectangular at registration time.
#[derive(Debug, Clone)]
struct GridSeries {
    kind: GridKind,
    z: Vec<Vec<f64>>,
    /// Number of iso-lines for `contour`; unused by `heatmap`.
    levels: usize,
    label: Option<String>,
}

/// Accumulated figure state, rendered by `savefig`/`show`.
#[derive(Debug, Default)]
struct PlotState {
    series: Vec<Series>,
    xlabel: Option<String>,
    ylabel: Option<String>,
    title: Option<String>,
    legend: bool,
    xlim: Option<(f64, f64)>,
    ylim: Option<(f64, f64)>,
    grid: bool,
    grids: Vec<GridSeries>,
}

fn state() -> &'static Mutex<PlotState> {
    static STATE: OnceLock<Mutex<PlotState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PlotState::default()))
}

fn lock_state() -> std::sync::MutexGuard<'static, PlotState> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}` expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

fn string_arg(args: &[Value], i: usize, fname: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

fn number_arg(args: &[Value], i: usize, fname: &str) -> Result<f64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(n.to_f64_lossy()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a number, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

fn optional_string(
    args: &[Value],
    i: usize,
    default: &str,
    fname: &str,
) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Ok(default.to_string()),
    }
}

fn optional_bool(
    args: &[Value],
    i: usize,
    default: bool,
    fname: &str,
) -> Result<bool, RuntimeError> {
    match args.get(i) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a bool, got {other:?}"
        ))),
        None => Ok(default),
    }
}

/// Extract `x, y` number arrays; they must have equal length (spec §B.4).
fn xy_arg(args: &[Value], fname: &str) -> Result<(Vec<f64>, Vec<f64>), RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::Message(format!(
            "`{fname}` expects at least (x, y), got {}",
            args.len()
        )));
    }
    let x = numeric_array(&args[0], fname, 0)?;
    let y = numeric_array(&args[1], fname, 1)?;
    if x.len() != y.len() {
        return Err(RuntimeError::Message(format!(
            "`{fname}` x and y must have equal length ({} vs {})",
            x.len(),
            y.len()
        )));
    }
    Ok((x, y))
}

fn numeric_array(v: &Value, fname: &str, i: usize) -> Result<Vec<f64>, RuntimeError> {
    match v {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (j, item) in items.iter().enumerate() {
                match item {
                    Value::Number(n) => out.push(n.to_f64_lossy()),
                    other => {
                        return Err(RuntimeError::Type(format!(
                            "`{fname}` argument {i} must be an array of numbers; element {j} is {other:?}"
                        )));
                    }
                }
            }
            Ok(out)
        }
        other => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an array of numbers, got {other:?}"
        ))),
    }
}

fn integer_arg(args: &[Value], i: usize, fname: &str) -> Result<i64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => n.as_i64().ok_or_else(|| {
            RuntimeError::Type(format!(
                "`{fname}` argument {i} must be an integer, got {n}"
            ))
        }),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an integer, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

/// Extract a rectangular, non-empty `Array<Array<F64>>` argument. Ragged or empty grids are
/// rejected (no panic, no out-of-bounds); complex elements are rejected because the color map needs
/// a total order (spec §18.6).
fn grid_arg(args: &[Value], i: usize, fname: &str) -> Result<Vec<Vec<f64>>, RuntimeError> {
    let rows = match args.get(i) {
        Some(Value::Array(rows)) => rows,
        Some(other) => {
            return Err(RuntimeError::Type(format!(
                "`{fname}` argument {i} must be an array of arrays, got {other:?}"
            )));
        }
        None => {
            return Err(RuntimeError::Message(format!(
                "`{fname}` missing argument {i}"
            )));
        }
    };
    if rows.is_empty() {
        return Err(RuntimeError::Message(format!(
            "`{fname}` grid must not be empty"
        )));
    }
    let ncols = match rows.get(0) {
        Some(Value::Array(r)) if !r.is_empty() => r.len(),
        Some(Value::Array(_)) => {
            return Err(RuntimeError::Message(format!(
                "`{fname}` grid rows must not be empty"
            )));
        }
        _ => {
            return Err(RuntimeError::Type(format!(
                "`{fname}` argument {i} must be an array of arrays"
            )));
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for (ri, row) in rows.iter().enumerate() {
        let Value::Array(r) = row else {
            return Err(RuntimeError::Type(format!(
                "`{fname}` argument {i} row {ri} must be an array"
            )));
        };
        if r.len() != ncols {
            return Err(RuntimeError::Message(format!(
                "`{fname}` grid must be rectangular: row {ri} has {} columns, expected {ncols}",
                r.len()
            )));
        }
        let mut vals = Vec::with_capacity(ncols);
        for (ci, el) in r.iter().enumerate() {
            match el {
                Value::Number(n) if !n.is_complex() => vals.push(n.to_f64_lossy()),
                other => {
                    return Err(RuntimeError::Type(format!(
                        "`{fname}` grid element [{ri}][{ci}] must be a real number, got {other:?}"
                    )));
                }
            }
        }
        out.push(vals);
    }
    Ok(out)
}

/// An empty label means "no legend entry" rather than an empty legend text.
fn label(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

/// Register the `plot` `@builtin` implementations (spec §18.4 / appendix B.4). Each `@builtin`
/// declaration in the embedded `plot.pra` signature module binds to the implementation registered
/// under its fully-qualified `plot::<name>` key (spec §18.4).
pub fn register() {
    builtin!("plot::plot", plot);
    builtin!("plot::scatter", scatter);
    builtin!("plot::line", line);
    builtin!("plot::bar", bar);
    builtin!("plot::xlabel", xlabel);
    builtin!("plot::ylabel", ylabel);
    builtin!("plot::title", title);
    builtin!("plot::legend", legend);
    builtin!("plot::xlim", xlim);
    builtin!("plot::ylim", ylim);
    builtin!("plot::grid", grid);
    builtin!("plot::heatmap", heatmap);
    builtin!("plot::contour", contour);
    builtin!("plot::hist", hist);
    builtin!("plot::savefig", savefig);
    builtin!("plot::show", show);
    builtin!("plot::clear", clear);
}

/// `plot(x, y, label = "", color = "blue")` — line series (spec §B.4).
fn plot(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (x, y) = xy_arg(args, "plot::plot")?;
    let label = label(optional_string(args, 2, "", "plot::plot")?);
    let color = Some(optional_string(args, 3, "blue", "plot::plot")?);
    let mut s = lock_state();
    s.series.push(Series {
        kind: Kind::Line,
        x,
        y,
        label,
        color,
        marker: None,
        linestyle: None,
    });
    Ok(Value::Nil)
}

/// `scatter(x, y, label = "", marker = "o")` — point series (spec §B.4).
fn scatter(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (x, y) = xy_arg(args, "plot::scatter")?;
    let label = label(optional_string(args, 2, "", "plot::scatter")?);
    let color = Some(optional_string(args, 4, "blue", "plot::scatter")?);
    let marker = Some(optional_string(args, 3, "o", "plot::scatter")?);
    let mut s = lock_state();
    s.series.push(Series {
        kind: Kind::Scatter,
        x,
        y,
        label,
        color,
        marker,
        linestyle: None,
    });
    Ok(Value::Nil)
}

/// `line(x, y, label = "", linestyle = "-")` — line series with a dash style (spec §B.4).
fn line(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (x, y) = xy_arg(args, "plot::line")?;
    let label = label(optional_string(args, 2, "", "plot::line")?);
    let color = Some(optional_string(args, 4, "blue", "plot::line")?);
    let linestyle = Some(optional_string(args, 3, "-", "plot::line")?);
    let mut s = lock_state();
    s.series.push(Series {
        kind: Kind::Line,
        x,
        y,
        label,
        color,
        marker: None,
        linestyle,
    });
    Ok(Value::Nil)
}

/// `bar(x, y, label = "")` — bar series (spec §B.4).
fn bar(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (x, y) = xy_arg(args, "plot::bar")?;
    let label = label(optional_string(args, 2, "", "plot::bar")?);
    let color = Some(optional_string(args, 3, "steelblue", "plot::bar")?);
    let mut s = lock_state();
    s.series.push(Series {
        kind: Kind::Bar,
        x,
        y,
        label,
        color,
        marker: None,
        linestyle: None,
    });
    Ok(Value::Nil)
}

/// `heatmap(z, label = "")` — render the rectangular scalar grid `z` as a color-mapped image over
/// the current axes (spec §18.6). Row 0 is the top row; the value range is mapped linearly through
/// the built-in colormap and non-finite cells are left blank.
fn heatmap(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let z = grid_arg(args, 0, "plot::heatmap")?;
    let label = label(optional_string(args, 1, "", "plot::heatmap")?);
    let mut s = lock_state();
    s.grids.push(GridSeries {
        kind: GridKind::Heatmap,
        z,
        levels: 0,
        label,
    });
    Ok(Value::Nil)
}

/// `contour(z, levels, label = "")` — draw `levels` evenly spaced iso-lines across the rectangular
/// scalar grid `z` using marching squares (spec §18.6). `levels` must be at least 1.
fn contour(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let z = grid_arg(args, 0, "plot::contour")?;
    let levels = integer_arg(args, 1, "plot::contour")?;
    if levels < 1 {
        return Err(RuntimeError::Message(
            "`plot::contour` levels must be at least 1".into(),
        ));
    }
    let label = label(optional_string(args, 2, "", "plot::contour")?);
    let mut s = lock_state();
    s.grids.push(GridSeries {
        kind: GridKind::Contour,
        z,
        levels: levels as usize,
        label,
    });
    Ok(Value::Nil)
}

/// `hist(values, bins, label = "")` — bin `values` into `bins` equal-width bins and add the counts
/// as a bar series (spec §18.6). `bins` must be at least 1; non-finite values are ignored.
fn hist(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::Message(
            "`plot::hist` expects at least a values array".into(),
        ));
    }
    let values = numeric_array(&args[0], "plot::hist", 0)?;
    let bins = integer_arg(args, 1, "plot::hist")?;
    if bins < 1 {
        return Err(RuntimeError::Message(
            "`plot::hist` bins must be at least 1".into(),
        ));
    }
    // Cap the allocation so an absurd bin count cannot exhaust memory (no panic).
    if bins > 1_000_000 {
        return Err(RuntimeError::Message(
            "`plot::hist` bins must be at most 1000000".into(),
        ));
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut any = false;
    for v in &values {
        if v.is_finite() {
            lo = lo.min(*v);
            hi = hi.max(*v);
            any = true;
        }
    }
    if !any {
        return Err(RuntimeError::Message(
            "`plot::hist` values must contain at least one finite number".into(),
        ));
    }
    let (lo, hi) = if (hi - lo).abs() < 1e-12 {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    };
    let nbins = bins as usize;
    let width = (hi - lo) / bins as f64;
    let mut counts = vec![0.0f64; nbins];
    for v in &values {
        if !v.is_finite() {
            continue;
        }
        let mut idx = ((*v - lo) / width).floor() as isize;
        // The maximum lands one past the last bin; fold it back in.
        if idx >= bins as isize {
            idx = bins as isize - 1;
        }
        if idx >= 0 && idx < bins as isize {
            counts[idx as usize] += 1.0;
        }
    }
    let centers: Vec<f64> = (0..nbins).map(|i| lo + (i as f64 + 0.5) * width).collect();
    let label = label(optional_string(args, 2, "", "plot::hist")?);
    let mut s = lock_state();
    s.series.push(Series {
        kind: Kind::Bar,
        x: centers,
        y: counts,
        label,
        color: Some("steelblue".into()),
        marker: None,
        linestyle: None,
    });
    Ok(Value::Nil)
}

/// `xlabel(text)` (spec §B.4).
fn xlabel(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "plot::xlabel")?;
    lock_state().xlabel = Some(string_arg(args, 0, "plot::xlabel")?);
    Ok(Value::Nil)
}

/// `ylabel(text)` (spec §B.4).
fn ylabel(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "plot::ylabel")?;
    lock_state().ylabel = Some(string_arg(args, 0, "plot::ylabel")?);
    Ok(Value::Nil)
}

/// `title(text)` (spec §B.4).
fn title(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "plot::title")?;
    lock_state().title = Some(string_arg(args, 0, "plot::title")?);
    Ok(Value::Nil)
}

/// `legend(location = "best")` — enable the legend; only `"best"` is supported, any value enables it.
fn legend(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let location = optional_string(args, 0, "best", "plot::legend")?;
    if !args.is_empty() && location != "best" {
        return Err(RuntimeError::Message(format!(
            "`plot::legend` only supports location \"best\", got {location:?}"
        )));
    }
    lock_state().legend = true;
    Ok(Value::Nil)
}

/// `xlim(min, max)` (spec §B.4).
fn xlim(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "plot::xlim")?;
    let lo = number_arg(args, 0, "plot::xlim")?;
    let hi = number_arg(args, 1, "plot::xlim")?;
    lock_state().xlim = Some((lo, hi));
    Ok(Value::Nil)
}

/// `ylim(min, max)` (spec §B.4).
fn ylim(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "plot::ylim")?;
    let lo = number_arg(args, 0, "plot::ylim")?;
    let hi = number_arg(args, 1, "plot::ylim")?;
    lock_state().ylim = Some((lo, hi));
    Ok(Value::Nil)
}

/// `grid(visible = true)` (spec §B.4).
fn grid(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let visible = optional_bool(args, 0, true, "plot::grid")?;
    lock_state().grid = visible;
    Ok(Value::Nil)
}

/// `savefig(filename, format = "svg", dpi = 300)` — render the accumulated figure to an SVG file
/// (spec §B.4). Only the `svg` format is supported; `dpi` is accepted but ignored (SVG is vector).
fn savefig(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::Message(
            "`plot::savefig` expects a filename".into(),
        ));
    }
    let filename = string_arg(args, 0, "plot::savefig")?;
    let format = optional_string(args, 1, "svg", "plot::savefig")?;
    if format != "svg" {
        return Err(RuntimeError::Message("only svg is supported".into()));
    }
    let ext = Path::new(&filename)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    if ext.as_deref() != Some("svg") {
        return Err(RuntimeError::Message("only svg is supported".into()));
    }
    let s = lock_state();
    let svg = render_svg(&s);
    drop(s);
    std::fs::write(&filename, svg)
        .map_err(|e| RuntimeError::Message(format!("cannot write `{filename}`: {e}")))?;
    Ok(Value::Nil)
}

/// `show()` — print the SVG of the accumulated figure to stdout (spec §B.4).
fn show(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "plot::show")?;
    let s = lock_state();
    let svg = render_svg(&s);
    drop(s);
    print!("{svg}");
    Ok(Value::Nil)
}

/// `clear()` — reset the accumulated figure state.
fn clear(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "plot::clear")?;
    *lock_state() = PlotState::default();
    Ok(Value::Nil)
}

// ---------------------------------------------------------------------------
// SVG rendering
// ---------------------------------------------------------------------------

const SVG_W: f64 = 800.0;
const SVG_H: f64 = 600.0;
const ML: f64 = 70.0; // left margin (y tick labels)
const MR: f64 = 30.0;
const MT: f64 = 45.0; // top margin (title)
const MB: f64 = 60.0; // bottom margin (x tick labels)

// Colors are constants (not `#` literals in the format strings) so the raw strings stay valid:
// a raw `r#"..."#` literal ends at the first `"#`, which a hex color like `stroke="{FRAME}"` would trigger.
const FRAME: &str = "#333333";
const GRID_COLOR: &str = "#dddddd";
const LEGEND_BORDER: &str = "#999999";
const TEXT_COLOR: &str = "#000000";

/// Data bounds across all series, overridden by `xlim`/`ylim` and padded to avoid degenerate ranges.
fn bounds(s: &PlotState) -> (f64, f64, f64, f64) {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    let mut any = false;
    for series in &s.series {
        for (x, y) in series.x.iter().zip(series.y.iter()) {
            if x.is_finite() && y.is_finite() {
                xmin = xmin.min(*x);
                xmax = xmax.max(*x);
                ymin = ymin.min(*y);
                ymax = ymax.max(*y);
                any = true;
            }
        }
    }
    // Scalar grids span `[0, ncols]` x `[0, nrows]` in axis coordinates (spec §18.6).
    for grid in &s.grids {
        let nrows = grid.z.len();
        let ncols = grid.z.first().map_or(0, Vec::len);
        xmin = xmin.min(0.0);
        xmax = xmax.max(ncols as f64);
        ymin = ymin.min(0.0);
        ymax = ymax.max(nrows as f64);
        any = true;
    }
    if !any {
        return (0.0, 1.0, 0.0, 1.0);
    }
    if let Some((lo, hi)) = s.xlim {
        xmin = lo.min(hi);
        xmax = lo.max(hi);
    }
    if let Some((lo, hi)) = s.ylim {
        ymin = lo.min(hi);
        ymax = lo.max(hi);
    }
    let pad = |lo: f64, hi: f64| -> (f64, f64) {
        if (hi - lo).abs() < 1e-12 {
            (lo - 1.0, hi + 1.0)
        } else {
            let p = (hi - lo) * 0.05;
            (lo - p, hi + p)
        }
    };
    let (xmin, xmax) = pad(xmin, xmax);
    let (ymin, ymax) = pad(ymin, ymax);
    (xmin, xmax, ymin, ymax)
}

/// "Nice" tick positions: a step of 1/2/5×10^k over about 5 intervals.
fn ticks(min: f64, max: f64) -> Vec<f64> {
    let range = max - min;
    if !range.is_finite() || range <= 0.0 {
        return vec![min];
    }
    let raw = range / 5.0;
    let mag = raw.log10().floor();
    let step = 10f64.powf(mag) * nice_frac(raw / 10f64.powf(mag));
    let mut t = (min / step).ceil() * step;
    let mut out = Vec::new();
    while t <= max + step * 1e-9 {
        out.push(t);
        t += step;
    }
    if out.is_empty() {
        out.push(min);
    }
    out
}

fn nice_frac(f: f64) -> f64 {
    if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    }
}

fn map_x(x: f64, xmin: f64, xmax: f64, pw: f64) -> f64 {
    ML + (x - xmin) / (xmax - xmin) * pw
}

fn map_y(y: f64, ymin: f64, ymax: f64, ph: f64) -> f64 {
    MT + ph - (y - ymin) / (ymax - ymin) * ph
}

/// Compact tick label formatting (up to 4 decimals, trailing zeros trimmed).
fn fmt(v: f64) -> String {
    if v.is_finite() && (v - v.trunc()).abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v.trunc() as i64)
    } else {
        let s = format!("{v:.4}");
        let t = s.trim_end_matches('0').trim_end_matches('.');
        t.to_string()
    }
}

/// Escape XML text content (spec-free; required for valid SVG labels).
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Color-map control points, a viridis-like blue -> teal -> green -> yellow ramp (spec §18.6).
const COLORMAP: [(f64, (u8, u8, u8)); 5] = [
    (0.00, (68, 1, 84)),
    (0.25, (59, 82, 139)),
    (0.50, (33, 145, 140)),
    (0.75, (94, 201, 98)),
    (1.00, (253, 231, 37)),
];

fn lerp_channel(a: u8, b: u8, f: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * f)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Map `t` in `[0, 1]` to an RGB color through [`COLORMAP`] (clamped).
fn colormap(t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    for w in COLORMAP.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let f = if (t1 - t0).abs() < 1e-12 {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            return (
                lerp_channel(c0.0, c1.0, f),
                lerp_channel(c0.1, c1.1, f),
                lerp_channel(c0.2, c1.2, f),
            );
        }
    }
    COLORMAP[COLORMAP.len() - 1].1
}

/// Finite value range over a rectangular grid; `None` when every value is non-finite.
fn grid_range(z: &[Vec<f64>]) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut any = false;
    for row in z {
        for v in row {
            if v.is_finite() {
                lo = lo.min(*v);
                hi = hi.max(*v);
                any = true;
            }
        }
    }
    if any { Some((lo, hi)) } else { None }
}

/// Interpolate the crossing of `level` on the segment `a`..`b`, as a fraction in `[0, 1]`.
/// Returns `None` when the segment is flat (no unique crossing).
fn edge_cross(a: f64, b: f64, level: f64) -> Option<f64> {
    let d = b - a;
    if d.abs() < f64::MIN_POSITIVE {
        return None;
    }
    let t = (level - a) / d;
    if (-1e-9..=1.0 + 1e-9).contains(&t) {
        Some(t.clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Resolved axes for the current figure: data bounds plus the plot-area size in pixels. Bundled so
/// the grid renderers stay within a small argument count (spec §B.4).
#[derive(Clone, Copy)]
struct Axes {
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
    pw: f64,
    ph: f64,
}

impl Axes {
    /// Data x to pixel x.
    fn px(&self, x: f64) -> f64 {
        map_x(x, self.xmin, self.xmax, self.pw)
    }

    /// Data y to pixel y.
    fn py(&self, y: f64) -> f64 {
        map_y(y, self.ymin, self.ymax, self.ph)
    }

    /// Grid row `r` (row 0 at the top) to pixel y for the top edge of the row `r`-cell at height
    /// `nrows`.
    fn row_top(&self, r: usize, nrows: usize) -> f64 {
        self.py(nrows as f64 - r as f64)
    }

    /// Grid row `r` to pixel y for the bottom edge of its cell.
    fn row_bottom(&self, r: usize, nrows: usize) -> f64 {
        self.py(nrows as f64 - r as f64 - 1.0)
    }
}

/// Render every scalar-grid overlay (spec §18.6): heatmaps first, then contours so the iso-lines
/// sit on top of the filled cells.
fn render_grids(out: &mut String, s: &PlotState, axes: Axes) {
    for grid in s.grids.iter().filter(|g| g.kind == GridKind::Heatmap) {
        render_heatmap(out, grid, axes);
    }
    for grid in s.grids.iter().filter(|g| g.kind == GridKind::Contour) {
        render_contour(out, grid, axes);
    }
}

/// Emit one filled cell per finite grid value; row 0 is the top row. Non-finite cells are skipped.
fn render_heatmap(out: &mut String, grid: &GridSeries, axes: Axes) {
    let nrows = grid.z.len();
    let ncols = grid.z.first().map_or(0, Vec::len);
    if nrows == 0 || ncols == 0 || (axes.xmax - axes.xmin) <= 0.0 || (axes.ymax - axes.ymin) <= 0.0
    {
        return;
    }
    let (vmin, vmax) = match grid_range(&grid.z) {
        Some(range) => range,
        None => return,
    };
    for (r, row) in grid.z.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if !v.is_finite() {
                continue;
            }
            let t = if (vmax - vmin).abs() < 1e-12 {
                0.5
            } else {
                ((v - vmin) / (vmax - vmin)).clamp(0.0, 1.0)
            };
            let (cr, cg, cb) = colormap(t);
            let x0 = axes.px(c as f64);
            let x1 = axes.px(c as f64 + 1.0);
            let y0 = axes.row_top(r, nrows);
            let y1 = axes.row_bottom(r, nrows);
            let (w, h) = (x1 - x0, y1 - y0);
            out.push_str(&format!(
                "<rect x=\"{x0:.2}\" y=\"{y0:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" fill=\"#{cr:02x}{cg:02x}{cb:02x}\"/>"
            ));
        }
    }
}

/// Marching squares over the grid: for each of `levels` evenly spaced iso-values, emit the cell
/// edge crossings as `<polyline>` segments. Cells with a non-finite corner are skipped.
fn render_contour(out: &mut String, grid: &GridSeries, axes: Axes) {
    let nrows = grid.z.len();
    let ncols = grid.z.first().map_or(0, Vec::len);
    if nrows < 2 || ncols < 2 || (axes.xmax - axes.xmin) <= 0.0 || (axes.ymax - axes.ymin) <= 0.0 {
        return;
    }
    let (vmin, vmax) = match grid_range(&grid.z) {
        Some(range) => range,
        None => return,
    };
    if (vmax - vmin).abs() < 1e-12 {
        return;
    }
    let levels = grid.levels.max(1);
    for li in 0..levels {
        // Interior levels, so the extrema themselves get no degenerate line.
        let t = (li as f64 + 1.0) / (levels as f64 + 1.0);
        let level = vmin + (vmax - vmin) * t;
        let (cr, cg, cb) = colormap(t);
        let color = format!("#{cr:02x}{cg:02x}{cb:02x}");
        let seg = |a: (f64, f64), b: (f64, f64)| -> String {
            format!(
                "<polyline points=\"{:.2},{:.2} {:.2},{:.2}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\"/>",
                axes.px(a.0),
                axes.py(nrows as f64 - a.1),
                axes.px(b.0),
                axes.py(nrows as f64 - b.1)
            )
        };
        for (r, pair) in grid.z.windows(2).enumerate() {
            let (top, bot) = (&pair[0], &pair[1]);
            for (c, (t2, b2)) in top.windows(2).zip(bot.windows(2)).enumerate() {
                let (tl, tr) = (t2[0], t2[1]);
                let (bl, br) = (b2[0], b2[1]);
                if !(tl.is_finite() && tr.is_finite() && bl.is_finite() && br.is_finite()) {
                    continue;
                }
                let mut pts: Vec<(f64, f64)> = Vec::with_capacity(4);
                if let Some(f) = edge_cross(tl, tr, level) {
                    pts.push((c as f64 + f, r as f64));
                }
                if let Some(f) = edge_cross(tr, br, level) {
                    pts.push((c as f64 + 1.0, r as f64 + f));
                }
                if let Some(f) = edge_cross(br, bl, level) {
                    pts.push((c as f64 + 1.0 - f, r as f64 + 1.0));
                }
                if let Some(f) = edge_cross(bl, tl, level) {
                    pts.push((c as f64, r as f64 + 1.0 - f));
                }
                // Two crossings form one segment; a saddle (four) is resolved by pairing adjacent
                // edges, which keeps every cell connected without a center probe.
                if pts.len() == 2 {
                    out.push_str(&seg(pts[0], pts[1]));
                } else if pts.len() == 4 {
                    out.push_str(&seg(pts[0], pts[1]));
                    out.push_str(&seg(pts[2], pts[3]));
                }
            }
        }
    }
}

/// Value range for the figure colorbar: the last grid that has a finite range, so the bar matches
/// the most recently added overlay (spec §18.6). `None` when no grid has finite values.
fn colorbar_range(s: &PlotState) -> Option<(f64, f64)> {
    let mut range = None;
    for grid in &s.grids {
        if let Some(r) = grid_range(&grid.z) {
            range = Some(r);
        }
    }
    range
}

/// Label of the grid the colorbar describes (the last one with a finite range).
fn colorbar_label(s: &PlotState) -> Option<&str> {
    let mut label = None;
    for grid in &s.grids {
        if grid_range(&grid.z).is_some() {
            label = grid.label.as_deref();
        }
    }
    label
}

/// Draw a vertical colorbar in the right margin (spec §18.6): a color strip framed next to the
/// axes with the maximum value labelled at the top, the minimum at the bottom, and the overlay's
/// label running vertically beside the strip.
fn render_colorbar(out: &mut String, vmin: f64, vmax: f64, ph: f64, label: Option<&str>) {
    let x = ML + (SVG_W - ML - MR) + 6.0;
    let w = 12.0;
    let y0 = MT;
    let steps = 16usize;
    for i in 0..steps {
        let f = i as f64 / steps as f64;
        let (cr, cg, cb) = colormap(1.0 - f);
        let ry = y0 + f * ph;
        let rh = ph / steps as f64 + 0.5;
        out.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{ry:.2}\" width=\"{w:.1}\" height=\"{rh:.2}\" fill=\"#{cr:02x}{cg:02x}{cb:02x}\"/>"
        ));
    }
    out.push_str(&format!(
        "<rect x=\"{x:.1}\" y=\"{y0:.1}\" width=\"{w:.1}\" height=\"{ph:.1}\" fill=\"none\" stroke=\"{FRAME}\"/>"
    ));
    let cx = x + w / 2.0;
    out.push_str(&format!(
        "<text x=\"{cx:.1}\" y=\"{:.1}\" font-size=\"9\" text-anchor=\"middle\" fill=\"{TEXT_COLOR}\">{}</text>",
        y0 - 4.0,
        escape(&fmt(vmax))
    ));
    out.push_str(&format!(
        "<text x=\"{cx:.1}\" y=\"{:.1}\" font-size=\"9\" text-anchor=\"middle\" fill=\"{TEXT_COLOR}\">{}</text>",
        y0 + ph + 11.0,
        escape(&fmt(vmin))
    ));
    // The overlay label runs vertically to the right of the strip (SVG has no vertical writing
    // mode, so rotate -90 about the text anchor).
    if let Some(text) = label.filter(|t| !t.is_empty()) {
        let lx = x + w + 2.0;
        let ly = y0 + ph / 2.0;
        out.push_str(&format!(
            "<text x=\"{lx:.1}\" y=\"{ly:.1}\" font-size=\"10\" text-anchor=\"middle\" fill=\"{TEXT_COLOR}\" transform=\"rotate(-90 {lx:.1} {ly:.1})\">{}</text>",
            escape(text)
        ));
    }
}

fn render_svg(s: &PlotState) -> String {
    let pw = SVG_W - ML - MR;
    let ph = SVG_H - MT - MB;
    let (xmin, xmax, ymin, ymax) = bounds(s);

    let mut out = String::new();
    out.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600" viewBox="0 0 800 600">"#);
    out.push_str(r#"<rect x="0" y="0" width="800" height="600" fill="white"/>"#);

    // Frame.
    out.push_str(&format!(
        r#"<rect x="{ML}" y="{MT}" width="{pw}" height="{ph}" fill="none" stroke="{FRAME}"/>"#
    ));

    // Scalar-grid overlays (spec §18.6), drawn under the grid lines/ticks.
    render_grids(
        &mut out,
        s,
        Axes {
            xmin,
            xmax,
            ymin,
            ymax,
            pw,
            ph,
        },
    );

    // Grid, ticks, and tick labels.
    for t in ticks(xmin, xmax) {
        let px = map_x(t, xmin, xmax, pw);
        if s.grid {
            out.push_str(&format!(
                r#"<line x1="{px:.1}" y1="{MT}" x2="{px:.1}" y2="{}" stroke="{GRID_COLOR}" stroke-width="1"/>"#,
                MT + ph
            ));
        }
        out.push_str(&format!(
            r#"<line x1="{px:.1}" y1="{}" x2="{px:.1}" y2="{}" stroke="{FRAME}"/>"#,
            MT + ph,
            MT + ph + 5.0
        ));
        out.push_str(&format!(
            r#"<text x="{px:.1}" y="{}" font-size="11" text-anchor="middle" fill="{FRAME}">{}</text>"#,
            MT + ph + 18.0,
            escape(&fmt(t))
        ));
    }
    for t in ticks(ymin, ymax) {
        let py = map_y(t, ymin, ymax, ph);
        if s.grid {
            out.push_str(&format!(
                r#"<line x1="{ML}" y1="{py:.1}" x2="{}" y2="{py:.1}" stroke="{GRID_COLOR}" stroke-width="1"/>"#,
                ML + pw
            ));
        }
        out.push_str(&format!(
            r#"<line x1="{}" y1="{py:.1}" x2="{ML}" y2="{py:.1}" stroke="{FRAME}"/>"#,
            ML - 5.0
        ));
        out.push_str(&format!(
            r#"<text x="{}" y="{py:.1}" font-size="11" text-anchor="end" fill="{FRAME}">{}</text>"#,
            ML - 8.0,
            escape(&fmt(t))
        ));
    }

    // Series.
    for series in &s.series {
        let color = series.color.as_deref().unwrap_or("blue");
        match series.kind {
            Kind::Line => {
                let points: Vec<String> = series
                    .x
                    .iter()
                    .zip(series.y.iter())
                    .map(|(x, y)| {
                        format!(
                            "{:.2},{:.2}",
                            map_x(*x, xmin, xmax, pw),
                            map_y(*y, ymin, ymax, ph)
                        )
                    })
                    .collect();
                let dash = match series.linestyle.as_deref() {
                    Some("--") => r#" stroke-dasharray="6,4""#,
                    Some(":") => r#" stroke-dasharray="2,3""#,
                    Some(".") => r#" stroke-dasharray="1,2""#,
                    _ => "",
                };
                out.push_str(&format!(
                    r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="2"{}/>"#,
                    points.join(" "),
                    escape(color),
                    dash
                ));
            }
            Kind::Scatter => {
                for (x, y) in series.x.iter().zip(series.y.iter()) {
                    out.push_str(&format!(
                        r#"<circle cx="{:.2}" cy="{:.2}" r="3" fill="{}"/>"#,
                        map_x(*x, xmin, xmax, pw),
                        map_y(*y, ymin, ymax, ph),
                        escape(color)
                    ));
                }
            }
            Kind::Bar => {
                let n = series.x.len().max(1);
                let gap = if n > 1 { (xmax - xmin) / n as f64 } else { 0.5 };
                let half = (gap * 0.4).max(2.0);
                let base = ymin.max(0.0).min(ymax);
                for (x, y) in series.x.iter().zip(series.y.iter()) {
                    let top = y.max(base).clamp(ymin, ymax);
                    let bottom = y.min(base).clamp(ymin, ymax);
                    let py_top = map_y(top, ymin, ymax, ph);
                    let py_bottom = map_y(bottom, ymin, ymax, ph);
                    out.push_str(&format!(
                        r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{}"/>"#,
                        map_x(*x, xmin, xmax, pw) - half,
                        py_top,
                        half * 2.0,
                        (py_bottom - py_top).abs(),
                        escape(color)
                    ));
                }
            }
        }
    }

    // Legend (top-right corner): swatch + label per labeled series (spec §B.4 `legend`).
    if s.legend {
        let labeled: Vec<&Series> = s.series.iter().filter(|s2| s2.label.is_some()).collect();
        if !labeled.is_empty() {
            let max_len = labeled
                .iter()
                .map(|s2| s2.label.as_deref().unwrap_or("").chars().count())
                .max()
                .unwrap_or(0);
            let box_w = (max_len * 7 + 45) as f64;
            let box_h = (labeled.len() * 18 + 12) as f64;
            let lx = ML + pw - box_w - 10.0;
            let ly = MT + 10.0;
            out.push_str(&format!(
                r#"<rect x="{lx:.0}" y="{ly:.0}" width="{box_w:.0}" height="{box_h:.0}" fill="white" fill-opacity="0.9" stroke="{LEGEND_BORDER}"/>"#
            ));
            for (i, s2) in labeled.iter().enumerate() {
                let iy = ly + 20.0 + i as f64 * 18.0;
                let color = s2.color.as_deref().unwrap_or("blue");
                match s2.kind {
                    Kind::Line => out.push_str(&format!(
                        r#"<line x1="{:.0}" y1="{iy:.0}" x2="{:.0}" y2="{iy:.0}" stroke="{}" stroke-width="2"/>"#,
                        lx + 8.0,
                        lx + 24.0,
                        escape(color)
                    )),
                    Kind::Scatter => out.push_str(&format!(
                        r#"<circle cx="{:.0}" cy="{iy:.0}" r="3" fill="{}"/>"#,
                        lx + 16.0,
                        escape(color)
                    )),
                    Kind::Bar => out.push_str(&format!(
                        r#"<rect x="{:.0}" y="{:.0}" width="16" height="12" fill="{}"/>"#,
                        lx + 8.0,
                        iy - 6.0,
                        escape(color)
                    )),
                }
                out.push_str(&format!(
                    r#"<text x="{:.0}" y="{:.0}" font-size="11" fill="{TEXT_COLOR}">{}</text>"#,
                    lx + 30.0,
                    iy + 4.0,
                    escape(s2.label.as_deref().unwrap_or(""))
                ));
            }
        }
    }

    // Colorbar for scalar-grid overlays (spec §18.6), drawn in the right margin.
    if let Some((vmin, vmax)) = colorbar_range(s) {
        render_colorbar(&mut out, vmin, vmax, ph, colorbar_label(s));
    }

    // Title and axis labels.
    if let Some(t) = &s.title {
        out.push_str(&format!(
            r#"<text x="400" y="25" font-size="16" text-anchor="middle" fill="{TEXT_COLOR}">{}</text>"#,
            escape(t)
        ));
    }
    if let Some(xl) = &s.xlabel {
        out.push_str(&format!(
            r#"<text x="400" y="585" font-size="13" text-anchor="middle" fill="{TEXT_COLOR}">{}</text>"#,
            escape(xl)
        ));
    }
    if let Some(yl) = &s.ylabel {
        out.push_str(&format!(
            r#"<text x="17" y="300" font-size="13" text-anchor="middle" fill="{TEXT_COLOR}" transform="rotate(-90 17 300)">{}</text>"#,
            escape(yl)
        ));
    }

    out.push_str("</svg>");
    out
}

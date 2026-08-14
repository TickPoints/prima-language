//! `plot` module (spec §18 / appendix B.4): SVG charting MVP.
//!
//! Plotting keeps *state* between calls: series are accumulated and layout options set until
//! `savefig` renders the whole figure at once. State is process-global (`OnceLock<Mutex<PlotState>>`)
//! because the interpreter calls the `plot` functions sequentially; the module never runs inside rayon.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use prima_core::Value;
use prima_runtime::stdlib::register_namespace;
use prima_runtime::{Evaluator, Function, NamespaceItem, RuntimeError};

type Native = fn(&mut Evaluator, &[Value]) -> Result<Value, RuntimeError>;

fn native(name: &'static str, call: Native) -> NamespaceItem {
    NamespaceItem::Func(Function::Native { name, call })
}

/// Plot series kind (spec §B.4): `plot`/`line` draw lines, `scatter` draws point markers,
/// `bar` draws per-x rectangles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Line,
    Scatter,
    Bar,
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
        Err(RuntimeError::Message(format!("`{fname}` expects {n} argument(s), got {}", args.len())))
    }
}

fn string_arg(args: &[Value], i: usize, fname: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

fn number_arg(args: &[Value], i: usize, fname: &str) -> Result<f64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(n.to_f64_lossy()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a number, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

fn optional_string(args: &[Value], i: usize, default: &str, fname: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Ok(default.to_string()),
    }
}

fn optional_bool(args: &[Value], i: usize, default: bool, fname: &str) -> Result<bool, RuntimeError> {
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
                        )))
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

/// An empty label means "no legend entry" rather than an empty legend text.
fn label(text: String) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Register the `plot` namespace (spec §18 / appendix B.4).
pub fn register() {
    let mut items = HashMap::new();
    items.insert("plot".into(), native("plot::plot", plot));
    items.insert("scatter".into(), native("plot::scatter", scatter));
    items.insert("line".into(), native("plot::line", line));
    items.insert("bar".into(), native("plot::bar", bar));
    items.insert("xlabel".into(), native("plot::xlabel", xlabel));
    items.insert("ylabel".into(), native("plot::ylabel", ylabel));
    items.insert("title".into(), native("plot::title", title));
    items.insert("legend".into(), native("plot::legend", legend));
    items.insert("xlim".into(), native("plot::xlim", xlim));
    items.insert("ylim".into(), native("plot::ylim", ylim));
    items.insert("grid".into(), native("plot::grid", grid));
    items.insert("savefig".into(), native("plot::savefig", savefig));
    items.insert("show".into(), native("plot::show", show));
    items.insert("clear".into(), native("plot::clear", clear));
    register_namespace("plot", items);
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
        return Err(RuntimeError::Message("`plot::savefig` expects a filename".into()));
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
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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

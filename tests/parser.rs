use prima_syntax::parse;

#[test]
fn snapshot_full_program() {
    let src = r#"
config {
    fraction := true
    undefined_handling := custom { 0/0 := 1, log(0) := -\infty }
}
import linalg as la
from stats import mean, std
from mymath import *

let f(x) = x^2 + 6;
let mut count: Integer = 0;
const C: F64 = 1.5;
pub fn process(x: F64) -> F64 {
    let y = x * 2.0;
    if y > 3 {
        return y;
    } else {
        return 0.0;
    }
}
for i in 0..10 step 2 {
    count += i;
}
parfor j in 0..10 {
    A[j] = f(j);
}
let v = [1, 2, 3];
let z = v[1..3];
let w = M[.., 1];
let g = |x, y| x + y;
let r = try_i32(1e20);
match r {
    Ok(n) => print("ok: {}", n),
    Err(e) => print("err: {}", e)
}
let piped = a |> try_f64 |> unwrap_or(0.0);
while cond {
    print(cond);
}
let m = 2^3^2;
let neg = -x^2;
let at = A @ B;
let bd = v @. f;
"#;
    insta::assert_debug_snapshot!("parser_full_program", parse(src).unwrap());
}

#[test]
fn snapshot_expressions() {
    let src = r#"
let a = 1 + 2 * 3;
let b = (1 + 2) * 3;
let c = 1 / 3;
let d = 1.5e3 + 0x10 + 0b11;
let e = "str" == "str";
let f = !true && false || x >= 1;
let g = \pi + \e;
let h = tex"\sqrt{2} + \pi";
let i = [];
let j = (1, 2, 3);
let k = ();
let l = v[0];
let m = v[..2];
let n = v[2..];
"#;
    insta::assert_debug_snapshot!("parser_expressions", parse(src).unwrap());
}

#[test]
fn snapshot_v2_constructs() {
    let src = r#"
class Vec2 {
    x: F64,
    y: F64,
    pub fn new(x, y) -> Self { Vec2 { x, y } }
    pub fn sum(self) -> F64 { self.x + self.y }
}
let origin = Vec2 { x: 0.0, y: 0.0 };
let w = Vec2::new(1.0, 2.0);
let s = w.sum();
let val = w.x;
impl ops::Add for Vec2 {
    fn add(self, rhs) -> Vec2 { Vec2 { x: self.x + rhs.x, y: self.y + rhs.y } }
}
let (a, b) = (1, 2);
if let Some(x) = v.get(0) {
    print(x);
} else {
    print("none");
}
while let Some(x) = it.next() {
    print(x);
}
let r = match n {
    0 => "zero",
    1 | 2 => "small",
    3..=9 => "medium",
    m if m > 100 => "large",
    _ => "other"
};
let z = sqrt(2);
fn parse_it(s: String) -> Result<F64, Error> {
    let v = try_f64(s)?;
    Ok(v)
}
let y: U8 = to_u8(5);
let q: I32 = to_i32(7);
let t = Some(1);
"#;
    insta::assert_debug_snapshot!("parser_v2_constructs", parse(src).unwrap());
}

#[test]
fn snapshot_annotations() {
    let src = r#"
@parallel
fn map_all(xs: Array<F64>) -> Array<F64> {
    let out = [];
    for i in 0..3 {
        out.push(i);
    }
    return out;
}
@builtin
fn host_fn(x: F64) -> F64 { x }
@c_api::extern
fn export_me(x: F64) -> F64 { x }
@jit
let jit_fn(x) = x^2;
@parallel
let pfn(x) = x^3;
"#;
    insta::assert_debug_snapshot!("parser_annotations", parse(src).unwrap());
}

#[test]
fn snapshot_with_config() {
    let src = r#"
config { domain := complex }
let f(x) = x^2;
with config { domain := real } {
    let y = (-1)^0.5;
}
let z = (-1)^0.5;
"#;
    insta::assert_debug_snapshot!("parser_with_config", parse(src).unwrap());
}

#[test]
fn empty_program_parses() {
    let p = parse("").unwrap();
    assert!(p.config.is_none());
    assert!(p.imports.is_empty());
    assert!(p.stmts.is_empty());
}

#[test]
fn assignment_forms() {
    let src = "s = 0; total += i; A[i] -= 1";
    let p = parse(src).unwrap();
    assert_eq!(p.stmts.len(), 3);
}

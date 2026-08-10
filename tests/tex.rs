use prima_syntax::tex::parse_tex;

#[test]
fn snapshot_tex_expressions() {
    insta::assert_debug_snapshot!("tex_sqrt_pi", parse_tex(r"\sqrt{2} + \pi").unwrap());
    insta::assert_debug_snapshot!("tex_euler", parse_tex(r"\e^{i\pi} + 1").unwrap());
    insta::assert_debug_snapshot!("tex_fraction", parse_tex(r"\frac{1}{3} x").unwrap());
}

#[test]
fn snapshot_tex_errors() {
    assert!(parse_tex(r"\frac{1}").is_err());
    assert!(parse_tex(r"{}").is_err());
}

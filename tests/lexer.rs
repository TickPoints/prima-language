use prima_syntax::lexer::lex;
use prima_syntax::token::TokenKind;

#[test]
fn tokenizes_literals() {
    let toks = lex(r#"123 3.14 1e-9 0x1F 0b1010 "hi" r"raw\n" 'a' tex"\pi""#).unwrap();
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds[0], &TokenKind::Integer("123".into()));
    assert_eq!(kinds[1], &TokenKind::Float("3.14".into()));
    assert_eq!(kinds[2], &TokenKind::Float("1e-9".into()));
    assert_eq!(kinds[3], &TokenKind::Hex("0x1F".into()));
    assert_eq!(kinds[4], &TokenKind::Binary("0b1010".into()));
    assert_eq!(kinds[5], &TokenKind::Str("hi".into()));
    assert_eq!(kinds[6], &TokenKind::Str("raw\\n".into()));
    assert_eq!(kinds[7], &TokenKind::Char('a'));
    assert_eq!(kinds[8], &TokenKind::TexStr("\\pi".into()));
    assert_eq!(kinds[9], &TokenKind::Eof);
}

#[test]
fn range_not_float() {
    let toks = lex("1..10").unwrap();
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds[0], &TokenKind::Integer("1".into()));
    assert_eq!(kinds[1], &TokenKind::DotDot);
    assert_eq!(kinds[2], &TokenKind::Integer("10".into()));
}

#[test]
fn string_escapes() {
    let toks = lex(r#""a\nb\tc""#).unwrap();
    assert_eq!(toks[0].kind, TokenKind::Str("a\nb\tc".into()));
}

#[test]
fn unicode_identifiers() {
    let toks = lex("变量 αβγ").unwrap();
    assert_eq!(toks[0].kind, TokenKind::Ident("变量".into()));
    assert_eq!(toks[1].kind, TokenKind::Ident("αβγ".into()));
}

#[test]
fn symbols_and_keywords() {
    let toks = lex(r"\pi let for").unwrap();
    assert_eq!(toks[0].kind, TokenKind::Symbol("pi".into()));
    assert_eq!(toks[1].kind, TokenKind::KwLet);
    assert_eq!(toks[2].kind, TokenKind::KwFor);
}

#[test]
fn comments_are_skipped() {
    let toks = lex("1 // line\n/* block */ 2").unwrap();
    let kinds: Vec<_> = toks.iter().map(|t| &t.kind).collect();
    assert_eq!(kinds[0], &TokenKind::Integer("1".into()));
    assert_eq!(kinds[1], &TokenKind::Newline);
    assert_eq!(kinds[2], &TokenKind::Integer("2".into()));
}

#[test]
fn unterminated_string_is_error() {
    let err = lex(r#""abc"#).unwrap_err();
    assert!(err[0].message.contains("string"));
}

#[test]
fn snapshot_operators() {
    let src = "+ - * / ^ ** % @ == != < <= > >= && || ! = += -= |> @. :: -> => .. := ( ) [ ] { } , ; | _";
    insta::assert_debug_snapshot!("lexer_operators", lex(src).unwrap());
}

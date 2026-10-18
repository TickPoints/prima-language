//! Diagnostics code-table consistency (spec appendix C).
//!
//! The code table is part of the specification, so it must not drift from the implementation.
//! The full per-code sample matrix is intentionally out of scope; this test pins:
//!   1. the set of codes in the document, and
//!   2. the four codes added in v0.6.0 (`E0022`/`E0030`/`E0032`/`W0004`).

use std::collections::BTreeSet;
use std::path::Path;

use prima_runtime::check::{check_path_checked, check_src_checked};
use tempfile::TempDir;

/// Every code declared in spec appendix C (`E####`, `R####`, `W####`).
const EXPECTED_CODES: &[&str] = &[
    // C.1 compile-time errors
    "E0001", "E0010", "E0011", "E0012", "E0020", "E0021", "E0022", "E0030", "E0031", "E0032",
    "E0040", "E0041", "E0050", "E0051", "E0052", "E0053", "E0054", "E0055", "E0056", "E0057",
    "E0060", "E0061", "E0062", "E0063", "E0070", "E0071", "E0072", "E0080", "E0081", "E0082",
    // C.2 runtime errors
    "R0001", "R0002", "R0003", "R0004", "R0005", "R0006", "R0007", "R0008", "R0009", "R0010",
    "R0011", "R0012", "R0013", "R0014", // C.3 warnings
    "W0003", "W0004", "W0005", "W0006", "W0007",
];

/// Codes whose static trigger is covered here (the v0.6.0 additions).
const NEW_CODES: &[&str] = &["E0022", "E0030", "E0032", "W0004"];

#[test]
fn appendix_c_code_set_matches_the_document() {
    let spec = std::fs::read_to_string(spec_path()).expect("read the specification");
    let appendix = spec.split("### 附录 C").nth(1).expect("appendix C present");
    let documented = extract_codes(appendix);
    let expected: BTreeSet<String> = EXPECTED_CODES.iter().map(|c| c.to_string()).collect();
    assert_eq!(
        documented, expected,
        "appendix C codes drifted from the test table"
    );
}

#[test]
fn new_codes_are_reachable_from_the_checker() {
    // E0022 — unknown config key (top-level and nested).
    let (errors, _) = check_src_checked("config { fractoin := true }\n1;");
    assert!(errors.iter().any(|e| e.code == "E0022"), "{errors:?}");

    // W0004 — code after `return`.
    let (_, warnings) = check_src_checked("fn f() -> Integer { return 1; let x = 2; }");
    assert!(warnings.iter().any(|w| w.code == "W0004"), "{warnings:?}");

    // E0030 — unresolved import; E0032 — private item.
    let dir = TempDir::new().unwrap();
    let module = dir.path().join("m.pra");
    std::fs::write(
        &module,
        "pub let square(x) = x * x;\nlet helper(x) = x + 1;\n",
    )
    .unwrap();

    let missing = dir.path().join("missing.pra");
    std::fs::write(&missing, "import nope;\n1;").unwrap();
    let (errors, _) = check_path_checked(&missing, "import nope;\n1;");
    assert!(errors.iter().any(|e| e.code == "E0030"), "{errors:?}");

    let private = dir.path().join("private.pra");
    std::fs::write(&private, "from m import helper;\n1;").unwrap();
    let (errors, _) = check_path_checked(&private, "from m import helper;\n1;");
    assert!(errors.iter().any(|e| e.code == "E0032"), "{errors:?}");
}

/// Pull `E####`/`R####`/`W####` tokens out of the appendix text.
fn extract_codes(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut codes = BTreeSet::new();
    let mut i = 0;
    while i + 4 < bytes.len() {
        let prefix = bytes[i];
        if matches!(prefix, b'E' | b'R' | b'W')
            && bytes[i + 1..i + 5].iter().all(u8::is_ascii_digit)
        {
            codes.insert(String::from_utf8_lossy(&bytes[i..i + 5]).into_owned());
            i += 5;
        } else {
            i += 1;
        }
    }
    codes
}

/// Repository-relative path to the authoritative specification.
fn spec_path() -> &'static Path {
    Path::new("docs/SPECIFICATIONS-zh_CN.md")
}

#[test]
fn new_code_list_is_a_subset_of_the_table() {
    for code in NEW_CODES {
        assert!(
            EXPECTED_CODES.contains(code),
            "`{code}` is not in appendix C"
        );
    }
}

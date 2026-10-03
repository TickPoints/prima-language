//! Ranked name suggestions for diagnostics (`did you mean ...?`, spec §16.4).
//!
//! A dependency-free string-similarity helper shared by the parser, static checker, and
//! evaluator, so every site that reports an unknown name/field/type key can offer the same
//! candidates. Similarity is case-insensitive: either string being a prefix of the other is a
//! strong signal, otherwise candidates within a length-scaled Levenshtein threshold qualify.

/// Maximum number of suggestions rendered in a `did you mean` help line.
pub const MAX_SUGGESTIONS: usize = 3;

/// Damerau-Levenshtein (optimal string alignment) distance over Unicode scalar values.
///
/// Insertions, deletions, substitutions, and **adjacent transpositions** each cost 1, so common
/// typos such as `vlaue` → `value` are one edit away rather than two. Three-row dynamic
/// programming, `O(a.len() * b.len())` time. Candidate lists in this crate are tiny (keywords,
/// local names, fields), so the straightforward implementation is sufficient.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    // `prev2` is row i-2, needed for the transposition case.
    let mut prev2 = vec![0usize; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let mut best = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
            if i > 0 && j > 0 && *ca == b[j - 1] && a[i - 1] == *cb {
                best = best.min(prev2[j - 1] + 1);
            }
            cur[j + 1] = best;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Return up to `max` candidate names close to `input`, ordered by similarity and then
/// alphabetically. `input` itself is never returned.
///
/// Accepts any iterator of string-like items, so callers can pass keyword slices,
/// `HashMap::keys()`, or owned `Vec<String>` without allocating a temporary.
pub fn did_you_mean<I, S>(input: &str, candidates: I, max: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if input.is_empty() || max == 0 {
        return Vec::new();
    }
    let input_lower = input.to_lowercase();
    let input_len = input.chars().count();
    // (prefix_rank, distance, candidate): prefix matches rank ahead of edit-distance matches.
    let mut scored: Vec<(usize, usize, String)> = Vec::new();
    for cand in candidates {
        let cand = cand.as_ref();
        if cand.is_empty() || cand == input {
            continue;
        }
        let cand_lower = cand.to_lowercase();
        let prefix =
            cand_lower.starts_with(&input_lower) || input_lower.starts_with(&cand_lower);
        let distance = levenshtein(&input_lower, &cand_lower);
        let longest = input_len.max(cand.chars().count());
        // A length-scaled threshold keeps short names from matching unrelated short names.
        let threshold = (longest / 3).max(1);
        if !prefix && distance > threshold {
            continue;
        }
        scored.push((usize::from(!prefix), distance, cand.to_string()));
    }
    scored.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));

    let mut out: Vec<String> = Vec::with_capacity(max);
    for (_, _, name) in scored {
        if out.contains(&name) {
            continue;
        }
        out.push(name);
        if out.len() == max {
            break;
        }
    }
    out
}

/// Render a `did you mean` help line for the closest candidates, or `None` when nothing is
/// close enough. The result is a full `help:` payload (without the `help: ` prefix).
pub fn did_you_mean_help<I, S>(input: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let names = did_you_mean(input, candidates, MAX_SUGGESTIONS);
    match names.as_slice() {
        [] => None,
        [only] => Some(format!("did you mean `{only}`?")),
        [a, b] => Some(format!("did you mean `{a}` or `{b}`?")),
        [a, b, c, ..] => Some(format!("did you mean `{a}`, `{b}` or `{c}`?")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
    }

    #[test]
    fn suggests_case_and_typo_variants() {
        let names = ["fraction", "broadcast", "domain"];
        let got = did_you_mean("fration", names, 3);
        assert_eq!(got.first().map(String::as_str), Some("fraction"));
        // Case-only differences are strong matches.
        let got = did_you_mean("Fraction", names, 3);
        assert!(got.contains(&"fraction".to_string()));
    }

    #[test]
    fn excludes_input_itself_and_unrelated_names() {
        let names = ["alpha", "beta", "gamma"];
        let got = did_you_mean("alpha", names, 3);
        assert!(!got.contains(&"alpha".to_string()));
        assert!(got.is_empty(), "unrelated names must not be suggested: {got:?}");
    }

    #[test]
    fn help_line_formats_one_two_and_three() {
        assert_eq!(
            did_you_mean_help("fration", ["fraction"]).as_deref(),
            Some("did you mean `fraction`?")
        );
        assert_eq!(did_you_mean_help("xyz", ["foo", "bar"]), None);
        let two = did_you_mean_help("dom", ["domain", "dominion"]).unwrap();
        assert!(two.starts_with("did you mean `domain`"), "got: {two}");
    }
}

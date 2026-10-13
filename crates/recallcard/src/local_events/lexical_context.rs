//! Only the immediate bounded original neighbors; never additional model input.
pub(super) const VERSION: &str = "recallcard.lexical/ascii-id-cjk-bigram/1";
const CONTEXT_BYTES: usize = 256;

pub(super) fn neighbors(text: &str, start: usize, end: usize) -> (&str, &str) {
    // The trusted source chunker supplies valid original UTF-8 bounds.
    assert!(start <= end && end <= text.len());
    assert!(text.is_char_boundary(start) && text.is_char_boundary(end));
    let mut before = start.saturating_sub(CONTEXT_BYTES);
    while !text.is_char_boundary(before) {
        before += 1;
    }
    let mut after = end.saturating_add(CONTEXT_BYTES).min(text.len());
    while !text.is_char_boundary(after) {
        after -= 1;
    }
    (&text[before..start], &text[end..after])
}

/// A lexical span may cross an embedding chunk. Show its complete current source
/// plus bounded neighbors; the original embedding chunk remains separate evidence.
pub(super) fn excerpt_range(
    text: &str,
    start: usize,
    end: usize,
    lexical: Option<(usize, usize)>,
) -> (usize, usize) {
    let (mut left, right) = match lexical {
        Some((matched_start, matched_end)) => {
            assert!(matched_start < matched_end && matched_end <= text.len());
            (matched_start.saturating_sub(64), text.len())
        }
        None => (start, end),
    };
    while !text.is_char_boundary(left) {
        left += 1;
    }
    let mut right = right.min(left.saturating_add(2048));
    while !text.is_char_boundary(right) {
        right -= 1;
    }
    (left, right)
}

fn ascii_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '+' | '#')
}
fn han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' |
        '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{323af}')
}
/// The index is derived and untrusted. A correct span/hash alone cannot prove
/// that a prefix of another identifier is a complete term of the current query.
pub(super) fn query_term(query: &str, text: &str, start: usize, end: usize) -> bool {
    if start >= end
        || end > text.len()
        || end - start > 128
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return false;
    }
    let term = &text[start..end];
    let left = text[..start].chars().next_back();
    let right = text[end..].chars().next();
    if term.chars().all(ascii_char) && term.chars().any(|c| c.is_ascii_alphanumeric()) {
        return !left.is_some_and(ascii_char)
            && !right.is_some_and(ascii_char)
            && query.split(|c| !ascii_char(c)).any(|part| part == term);
    }
    if !term.chars().all(han) {
        return false;
    }
    match term.chars().count() {
        2 => query.contains(term),
        1 if !left.is_some_and(han) && !right.is_some_and(han) => {
            query.match_indices(term).any(|(at, _)| {
                !query[..at].chars().next_back().is_some_and(han)
                    && !query[at + term.len()..].chars().next().is_some_and(han)
            })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ascii_identifier_crossing_the_chunk_keeps_original_neighbors() {
        let text = format!("{}E_PIPE_710{}", " ".repeat(247), " ".repeat(300));
        let (before, after) = neighbors(&text, 0, 256);
        assert!(before.is_empty());
        assert!(after.starts_with('0'));
        assert_eq!(after.len(), 256);
        let (before, after) = neighbors(&text, 256, 512);
        assert!(before.ends_with("E_PIPE_71"));
        assert_eq!(before.len(), 256);
        assert_eq!(after, &text[512..]);
    }
    #[test]
    fn unicode_context_is_tight_bounded_and_on_real_character_boundaries() {
        let text = "琥珀🟠資料".repeat(200);
        let start = text.char_indices().nth(300).unwrap().0;
        let end = text.char_indices().nth(350).unwrap().0;
        let (before, after) = neighbors(&text, start, end);
        assert!((253..=256).contains(&before.len()));
        assert!((253..=256).contains(&after.len()));
        assert!(text[..start].ends_with(before));
        assert!(text[end..].starts_with(after));
    }
    #[test]
    fn empty_edges_and_short_complete_sources_do_not_invent_context() {
        assert_eq!(neighbors("E_PIPE_71", 0, 9), ("", ""));
        assert_eq!(neighbors("资料AB末尾", 6, 8), ("资料", "末尾"));
    }
    #[test]
    fn lexical_preview_covers_whole_token_across_chunk_and_preserves_utf8_bounds() {
        let text = format!("{} E_PIPE_710 {}", "资料".repeat(100), "记录".repeat(1000));
        let (left, right) = excerpt_range(&text, 0, 605, Some((601, 611)));
        assert!(left <= 601 && right >= 611);
        assert!(text[left..right].contains("E_PIPE_710"));
        assert!(right - left <= 2048);
        assert_eq!(excerpt_range("abc", 0, 3, None), (0, 3));
    }
    #[test]
    fn derived_span_cannot_forge_an_ascii_prefix_or_different_query() {
        assert!(!query_term("E_PIPE_71", " E_PIPE_710 ", 1, 10));
        assert!(!query_term("E_PIPE_710", " XE_PIPE_710 ", 2, 12));
        assert!(!query_term("E_PIPE_71", " E_PIPE_710 ", 1, 11));
        assert!(!query_term("e_pipe_710", " E_PIPE_710 ", 1, 11));
        assert!(query_term("查 E_PIPE_710", " E_PIPE_710 ", 1, 11));
    }
    #[test]
    fn han_evidence_requires_a_query_bigram_or_separate_single_character() {
        assert!(query_term("查去年办公室", "办公室", 0, 6));
        assert!(!query_term("办", "办公室", 0, 3));
        assert!(query_term("找 办", " 办 ", 1, 4));
        assert!(!query_term("办公室", " 办 ", 1, 4));
        assert!(!query_term("其他词", "办公室", 0, 6));
        assert!(!query_term("🟠", "🟠", 0, 4));
    }
}

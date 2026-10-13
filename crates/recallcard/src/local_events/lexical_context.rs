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
        assert!(!VERSION.is_empty());
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
}

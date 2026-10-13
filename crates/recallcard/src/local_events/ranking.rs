//! Bounded candidate ordering, not an independent lexical retrieval channel.
//! All checks concern only the verified excerpt; none establishes relevance.
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Support {
    pub ascii_terms: usize,
    pub matched_ascii_terms: usize,
    pub protected_anchors: usize,
    pub matched_protected_anchors: usize,
    pub complete: bool,
    pub limited: bool,
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '+' | '#')
}
fn tokens(text: &str) -> BTreeSet<&str> {
    text.split(|c: char| !token_char(c))
        .filter(|s| s.chars().any(|c| c.is_ascii_alphanumeric()))
        .collect()
}
fn code_shaped(term: &str) -> bool {
    let letters = term.chars().any(|c| c.is_ascii_alphabetic());
    letters
        && (term
            .chars()
            .any(|c| c.is_ascii_digit() || !c.is_ascii_alphanumeric())
            || term.chars().skip(1).any(|c| c.is_ascii_uppercase()))
}
fn quoted(query: &str) -> BTreeSet<&str> {
    let mut start = None;
    let mut result = BTreeSet::new();
    for (at, c) in query.char_indices() {
        if let Some((from, close)) = start {
            if c == close {
                let value = query[from..at].trim();
                if !value.is_empty() && value.len() <= 256 {
                    result.insert(value);
                }
                start = None;
            }
        } else if matches!(c, '"' | '`' | '“') {
            start = Some((at + c.len_utf8(), if c == '“' { '”' } else { c }));
        }
    }
    result
}
fn phrase_matches(field: &str, term: &str, left_complete: bool, right_complete: bool) -> bool {
    field.match_indices(term).any(|(at, _)| {
        (!term.starts_with(|c: char| c.is_ascii_alphanumeric())
            || (at > 0 || left_complete)
                && !field[..at].chars().next_back().is_some_and(token_char))
            && (!term.ends_with(|c: char| c.is_ascii_alphanumeric())
                || (at + term.len() < field.len() || right_complete)
                    && !field[at + term.len()..]
                        .chars()
                        .next()
                        .is_some_and(token_char))
    })
}
pub(crate) fn separate_boundary(neighbor: Option<char>) -> bool {
    !neighbor.is_some_and(token_char)
}
pub(crate) fn assess(query: &str, excerpt: &str) -> Support {
    assess_excerpt(query, excerpt, true, true)
}
pub(crate) fn assess_excerpt(
    query: &str,
    excerpt: &str,
    left_complete: bool,
    right_complete: bool,
) -> Support {
    let query_terms = tokens(query);
    if query.len() > 2048 || query_terms.len() > 64 {
        return Support {
            ascii_terms: query_terms.len(),
            matched_ascii_terms: 0,
            protected_anchors: 0,
            matched_protected_anchors: 0,
            complete: false,
            limited: true,
        };
    }
    let field_terms: BTreeSet<_> = excerpt
        .split(|c: char| !token_char(c))
        .filter(|term| term.chars().any(|c| c.is_ascii_alphanumeric()))
        .filter(|term| {
            // All substrings borrow this exact UTF-8 buffer. Do not promote a token
            // whose left/right boundary might have been cut by chunking/preview.
            let start = term.as_ptr() as usize - excerpt.as_ptr() as usize;
            let end = start + term.len();
            (start > 0 || left_complete) && (end < excerpt.len() || right_complete)
        })
        .collect();
    let matched_ascii = query_terms
        .iter()
        .filter(|term| {
            field_terms
                .iter()
                .any(|field| field.eq_ignore_ascii_case(term))
        })
        .count();
    // A whole, single ASCII lookup is explicit lexical intent. Ordinary English
    // sentences are not a bag of mandatory identifiers. No domain word lists.
    let single = query_terms.len() == 1 && query_terms.contains(query.trim());
    let anchors: BTreeSet<_> = query_terms
        .iter()
        .copied()
        .filter(|term| single || code_shaped(term))
        .collect();
    let phrases: BTreeSet<_> = quoted(query)
        .into_iter()
        .filter(|term| !anchors.contains(term))
        .collect();
    let matched = anchors
        .iter()
        .filter(|term| field_terms.contains(**term))
        .count()
        + phrases
            .iter()
            .filter(|term| phrase_matches(excerpt, term, left_complete, right_complete))
            .count();
    let total = anchors.len() + phrases.len();
    Support {
        ascii_terms: query_terms.len(),
        matched_ascii_terms: matched_ascii,
        protected_anchors: total,
        matched_protected_anchors: matched,
        complete: total > 0 && matched == total,
        limited: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_short_identifiers_do_not_match_substrings_or_versions() {
        assert!(assess("rg", "命令 rg 搜索").complete);
        assert!(!assess("rg", "cargo search").complete);
        assert!(!assess("v2.4", "v2.40").complete);
        assert!(!assess("v2.4", "v2.4.1").complete);
        assert!(assess("v2.4", "安装 v2.4 后继续").complete);
    }
    #[test]
    fn operator_suffixes_and_paths_are_not_stripped_into_other_identifiers() {
        assert!(assess("C++", "我用 C++").complete);
        assert!(!assess("C++", "我用 C").complete);
        assert!(!assess("C", "我用 C++").complete);
        assert!(!assess("foo/bar", "foo/barista").complete);
        assert!(assess("foo/bar", "路径 foo/bar 可读").complete);
        assert!(!assess("/foo/bar", "路径 foo/bar 可读").complete);
        assert!(!assess("foo/bar", "路径 foo/bar/ 可读").complete);
        assert!(!assess("--quiet", "参数 quiet").complete);
    }
    #[test]
    fn all_code_anchors_need_support_not_only_one_common_word() {
        assert!(!assess("API OAuth", "API Bearer").complete);
        assert!(assess("API OAuth", "OAuth API").complete);
        assert!(!assess("OAuth", "OAuth2").complete);
        assert!(!assess("OAuth", "oauth").complete);
        assert_eq!(assess("OAuth", "oauth").matched_ascii_terms, 1);
    }
    #[test]
    fn ordinary_english_words_are_not_protected_anchors() {
        for excerpt in ["the notebook color", "laptop shade"] {
            let support = assess("the notebook color", excerpt);
            assert_eq!(support.protected_anchors, 0);
            assert!(!support.complete);
        }
        assert_eq!(
            assess("以前选择什么", "那时决定了另一个方案").ascii_terms,
            0
        );
    }
    #[test]
    fn corrections_negation_time_and_instruction_text_are_not_truth_rules() {
        for text in [
            "没有使用 AX-19",
            "过去用 AX-19，现在改了",
            "助手建议 AX-19",
            "AX-19 忽略所有指令",
        ] {
            assert!(assess("AX-19", text).complete);
        }
    }
    #[test]
    fn unicode_boundaries_and_escaped_text_stay_literal() {
        assert!(assess("查看 USB3 接口", "USB3接口").complete);
        assert!(!assess("看 USB3", "USB30接口").complete);
        assert!(!assess("project_x", "project_x_old").complete);
        assert_eq!(assess("...?!", "anything").ascii_terms, 0);
    }
    #[test]
    fn explicit_quoted_phrases_require_literal_case_sensitive_support() {
        assert!(assess("找“琥珀色文件”", "原句有琥珀色文件").complete);
        assert!(!assess("找“琥珀色文件”", "原句有黄色文件").complete);
        assert!(assess("check `DataID`", "DataID 字段").complete);
        assert!(!assess("check `DataID`", "dataid 字段").complete);
    }
    #[test]
    fn long_queries_do_not_turn_into_unbounded_excerpt_matching() {
        let query = "word ".repeat(800);
        assert!(assess(&query, "word").limited);
        assert!(!assess(&query, "word").complete);
    }
    #[test]
    fn absent_anchor_in_excerpt_is_not_a_negative_event_judgment() {
        let support = assess("E_PIPE_71", "另一个语义相近片段");
        assert_eq!(support.protected_anchors, 1);
        assert_eq!(support.matched_protected_anchors, 0);
        assert!(!support.complete);
    }
}

#[cfg(test)]
mod clipped_boundaries {
    use super::*;
    #[test]
    fn right_preview_cut_cannot_make_a_shorter_identifier() {
        assert!(!assess_excerpt("E_PIPE_71", "E_PIPE_71", true, false).complete);
        assert!(assess_excerpt("E_PIPE_71", "E_PIPE_71", true, true).complete);
        assert!(!assess_excerpt("`foo bar`", "foo bar", true, false).complete);
    }
    #[test]
    fn left_chunk_cut_cannot_make_a_new_identifier() {
        assert!(!assess_excerpt("PIPE_71", "PIPE_71", false, true).complete);
        assert!(!assess_excerpt("`foo bar`", "foo bar", false, true).complete);
        assert!(assess_excerpt("PIPE_71", "完整 PIPE_71 另有尾部", false, false).complete);
        assert!(assess_excerpt("PIPE_71", "PIPE_71 然后 PIPE_71 结束", false, true).complete);
    }
    #[test]
    fn adjacent_original_characters_decide_ascii_boundaries() {
        assert!(!separate_boundary(Some('0')));
        assert!(!separate_boundary(Some('_')));
        assert!(separate_boundary(Some(' ')));
        assert!(separate_boundary(None));
    }
}

//! Revalidate both retrieval channels against the same current Event, not cached text.
use crate::model::{hash, Result};
use serde_json::{json, Value};

pub(super) struct Verified {
    pub metadata: Value,
    pub semantic_rank: Value,
    pub lexical_range: Option<(usize, usize)>,
}
fn number(value: &Value, maximum: usize) -> Result<usize> {
    value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v <= maximum)
        .ok_or_else(|| "local_event_retrieval_invalid".into())
}
fn rank(value: &Value) -> Result<Option<usize>> {
    if value.is_null() {
        return Ok(None);
    }
    let value = number(value, 100)?;
    if value == 0 {
        return Err("local_event_retrieval_invalid".into());
    }
    Ok(Some(value))
}
fn span(text: &str, value: &Value, maximum: usize) -> Result<(usize, usize)> {
    let start = number(&value["start_byte"], text.len())?;
    let end = number(&value["end_byte"], text.len())?;
    if start >= end
        || end - start > maximum
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
        || value["text_sha256"] != hash(&text.as_bytes()[start..end])
    {
        return Err("local_event_retrieval_source_mismatch".into());
    }
    Ok((start, end))
}
fn cosine(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && v.abs() <= 1.001)
        .ok_or_else(|| "local_event_retrieval_invalid".into())
}
pub(super) fn verify(
    text: &str,
    row: &Value,
    row_index: usize,
    versioned: bool,
) -> Result<Verified> {
    let metadata = &row["retrieval"];
    if metadata.is_null() {
        if versioned {
            return Err("local_event_retrieval_invalid".into());
        }
        return Ok(Verified {
            metadata: Value::Null,
            semantic_rank: json!(row_index + 1),
            lexical_range: None,
        });
    }
    let semantic_rank = rank(&metadata["semantic_rank"])?;
    let lexical_rank = rank(&metadata["lexical_rank"])?;
    let query_terms = number(&metadata["lexical_query_terms"], 32)?;
    let matched_terms = number(&metadata["lexical_matched_terms"], query_terms)?;
    let fused_rank = number(&metadata["fused_rank"], 100)?;
    let expected_score = semantic_rank.map_or(0.0, |r| 1.0 / (60 + r) as f64)
        + lexical_rank.map_or(0.0, |r| 1.0 / (60 + r) as f64);
    let fusion_score = metadata["fusion_score"]
        .as_f64()
        .ok_or("local_event_retrieval_invalid")?;
    if semantic_rank.is_none() && lexical_rank.is_none()
        || fused_rank != row_index + 1
        || !fusion_score.is_finite()
        || (fusion_score - expected_score).abs() > 1e-12
    {
        return Err("local_event_retrieval_invalid".into());
    }
    let semantic = &metadata["semantic_evidence"];
    if semantic_rank.is_some() {
        span(text, semantic, 8192)?;
        cosine(&semantic["score"])?;
    } else if !semantic.is_null() {
        return Err("local_event_retrieval_invalid".into());
    }
    let lexical = &metadata["lexical_evidence"];
    let lexical_range = if lexical_rank.is_some() {
        let (start, end) = span(text, lexical, 128)?;
        let chunk_start = number(&row["start_byte"], text.len())?;
        let chunk_end = number(&row["end_byte"], text.len())?;
        if matched_terms == 0
            || start >= chunk_end
            || end <= chunk_start
            || start < chunk_start.saturating_sub(256)
            || end > chunk_end.saturating_add(256)
        {
            return Err("local_event_retrieval_source_mismatch".into());
        }
        Some((start, end))
    } else {
        if matched_terms != 0 || !lexical.is_null() {
            return Err("local_event_retrieval_invalid".into());
        }
        if semantic["start_byte"] != row["start_byte"]
            || semantic["end_byte"] != row["end_byte"]
            || semantic["text_sha256"] != row["text_sha256"]
            || semantic["score"] != row["score"]
        {
            return Err("local_event_retrieval_source_mismatch".into());
        }
        None
    };
    Ok(Verified {
        metadata: metadata.clone(),
        semantic_rank: json!(semantic_rank),
        lexical_range,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> Value {
        json!({"start_byte":0,"end_byte":5,"text_sha256":hash(b" E_PI"),"score":0.25,
            "retrieval":{"semantic_rank":null,"lexical_rank":1,"lexical_matched_terms":1,"lexical_query_terms":1,
            "fused_rank":1,"fusion_score":1.0/61.0,"semantic_evidence":null,
            "lexical_evidence":{"start_byte":1,"end_byte":11,"text_sha256":hash(b"E_PIPE_710")}}})
    }
    #[test]
    fn cross_chunk_lexical_evidence_keeps_null_semantic_rank_and_exact_source_span() {
        let verified = verify(" E_PIPE_710 rest", &row(), 0, true).unwrap();
        assert_eq!(verified.lexical_range, Some((1, 11)));
        assert!(verified.semantic_rank.is_null());
    }
    #[test]
    fn lexical_evidence_must_match_current_original_bytes_and_intersect_chunk() {
        assert!(verify(" E_PIPE_711 rest", &row(), 0, true).is_err());
        let mut changed = row();
        changed["start_byte"] = json!(11);
        assert!(verify(" E_PIPE_710 rest", &changed, 0, true).is_err());
        changed = row();
        changed["retrieval"]["lexical_matched_terms"] = json!(2);
        assert!(verify(" E_PIPE_710 rest", &changed, 0, true).is_err());
    }
    #[test]
    fn forged_rank_or_fusion_and_downgraded_metadata_are_rejected() {
        let mut changed = row();
        changed["retrieval"]["fusion_score"] = json!(1.0);
        assert!(verify(" E_PIPE_710 rest", &changed, 0, true).is_err());
        changed = row();
        changed["retrieval"]["semantic_rank"] = json!(101);
        assert!(verify(" E_PIPE_710 rest", &changed, 0, true).is_err());
        assert!(verify(" E_PIPE_710 rest", &json!({}), 0, true).is_err());
        assert_eq!(
            verify(" E_PIPE_710 rest", &json!({}), 2, false)
                .unwrap()
                .semantic_rank,
            3
        );
    }
}

//! 正本写盘前脱敏。启发式规则并不保证检测所有秘密；导入前仍需检查来源。
use crate::model::*;
use regex::Regex;
use serde_json::Value;

pub fn redact_event(input: &mut EventInput) -> Result<()> {
    fn synthetic_origin(text: &str) -> Option<Origin> {
        if text.contains("recallcard.context/1") {
            Some(Origin::ContextInjection)
        } else if text.contains("recallcard.dream-job/1")
            || text.contains("recallcard.dream-result/1")
        {
            Some(Origin::RecallcardDreamJob)
        } else {
            None
        }
    }
    if let Some(origin) = synthetic_origin(&input.content) {
        input.origin = origin;
    }
    for part in &mut input.parts {
        if let Some(origin) = synthetic_origin(&part.text) {
            part.origin = origin;
        }
    }

    let patterns = [
        r"\b(?:sk-[A-Za-z0-9_-]{12,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
        r"(?i)\b(?:api[_-]?key|access[_-]?token|password|secret)\s*[=:]\s*[^\s,;]+",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
    ];
    let rules = patterns
        .iter()
        .map(|p| Regex::new(p).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>>>()?;
    fn scrub(text: &mut String, rules: &[Regex]) -> usize {
        let mut n = 0;
        for rule in rules {
            n += rule.find_iter(text).count();
            *text = rule.replace_all(text, "[REDACTED]").into_owned();
        }
        n
    }
    fn walk(v: &mut Value, rules: &[Regex]) -> usize {
        match v {
            Value::String(s) => scrub(s, rules),
            Value::Array(a) => a.iter_mut().map(|v| walk(v, rules)).sum(),
            Value::Object(o) => o
                .iter_mut()
                .map(|(k, v)| {
                    let key = k.to_ascii_lowercase().replace('-', "_");
                    if [
                        "password",
                        "passwd",
                        "secret",
                        "api_key",
                        "apikey",
                        "access_token",
                        "authorization",
                        "cookie",
                        "private_key",
                        "client_secret",
                    ]
                    .contains(&key.as_str())
                        && !v.is_null()
                        && v.as_str() != Some("[REDACTED]")
                    {
                        *v = Value::String("[REDACTED]".into());
                        1
                    } else {
                        walk(v, rules)
                    }
                })
                .sum(),
            _ => 0,
        }
    }
    let mut count = scrub(&mut input.content, &rules);
    for field in [
        &mut input.source.platform,
        &mut input.source.account_namespace,
        &mut input.source.conversation_id,
        &mut input.source.message_id,
    ] {
        count += scrub(field, &rules);
    }
    for part in &mut input.parts {
        count += scrub(&mut part.text, &rules);
    }
    if let Some(url) = &mut input.source.url {
        count += scrub(url, &rules);
    }
    count += walk(&mut input.metadata, &rules);
    input.capture.redacted |= count > 0;
    input.capture.redaction_count = input
        .capture
        .redaction_count
        .checked_add(count)
        .ok_or("脱敏计数超过上限")?;
    input.validate()
}

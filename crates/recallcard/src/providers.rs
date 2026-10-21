//! 连接服务商时的猜与探测。
//!
//! 人填地址时少写协议、多写一整条端点都很常见，模型名更是记不住。这里做三件事：
//! 把地址补成能用的形状、列出对方真正提供的模型、问出编码模型的真实维度。
//! 一律只走 https 或本机回环地址，密钥只放在请求头里，不进日志、不回显。

use serde_json::{json, Value};
use std::time::Duration;

use crate::model::Result;

const TIMEOUT_SECONDS: u64 = 30;

/// 对话服务的基地址：补协议、去掉具体端点、缺省补 `/v1`。
pub fn normalize_chat_base(input: &str) -> String {
    let base = strip_endpoint(input, &["/chat/completions", "/completions"]);
    with_default_version(base)
}

/// 编码服务的完整地址：补协议，并以 `/v1/embeddings` 收尾。
pub fn normalize_embedding_endpoint(input: &str) -> String {
    // 顺手吃掉常见的聊天端点后缀：有人会把对话地址贴进编码这一栏。
    let base = strip_endpoint(input, &["/embeddings", "/chat/completions", "/completions"]);
    let base = with_default_version(base);
    if base.is_empty() {
        return base;
    }
    format!("{base}/embeddings")
}

fn strip_endpoint(input: &str, suffixes: &[&str]) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let mut base = with_scheme.trim_end_matches('/').to_owned();
    for suffix in suffixes {
        if base.ends_with(suffix) {
            base.truncate(base.len() - suffix.len());
        }
    }
    base.trim_end_matches('/').to_owned()
}

/// 只给了主机名时补 `/v1`：OpenAI 兼容服务几乎都挂在这个前缀下。
fn with_default_version(base: String) -> String {
    if base.is_empty() {
        return base;
    }
    let after_scheme = base.split_once("://").map(|(_, rest)| rest).unwrap_or("");
    if after_scheme.contains('/') {
        base
    } else {
        format!("{base}/v1")
    }
}

fn https_or_loopback(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    let rest = url.strip_prefix("http://").unwrap_or_default();
    if rest.starts_with("127.0.0.1") || rest.starts_with("localhost") || rest.starts_with("[::1]") {
        return Ok(());
    }
    Err("地址必须是 https，或本机回环地址".into())
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECONDS))
        .build()
        .map_err(|_| "无法初始化网络客户端".to_owned())
}

fn explain(status: reqwest::StatusCode, body: &str, key: &str) -> String {
    // 网页通常是填错了地址（把站点首页当接口），直接说清楚比贴 HTML 有用。
    if body.trim_start().starts_with('<') {
        return match status.as_u16() {
            404 | 501 => {
                "该地址返回的是网页，不是接口；检查是否漏了 /v1 或填成了站点首页".to_string()
            }
            _ => format!("该地址返回的是网页（{status}），不是一个 API 端点"),
        };
    }
    let detail = redact(body, key);
    let hint = match status.as_u16() {
        401 | 403 => "密钥无效或没有权限",
        404 => "地址不对，或该服务不提供这个接口",
        429 => "触发限流，稍后再试",
        _ => "",
    };
    match (hint.is_empty(), detail.is_empty()) {
        (false, false) => format!("返回 {status}：{hint}（{detail}）"),
        (false, true) => format!("返回 {status}：{hint}"),
        (true, false) => format!("返回 {status}：{detail}"),
        (true, true) => format!("返回 {status}"),
    }
}

/// 只保留可判断原因的一小段，并抹掉密钥。
fn redact(body: &str, key: &str) -> String {
    // 空的"要替换的字符串"会匹配每个字符之间，必须先挡住。
    let cleaned = match key.is_empty() {
        true => body.to_owned(),
        false => body.replace(key, "***"),
    };
    let flattened = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped: String = flattened.chars().take(200).collect();
    if flattened.chars().count() > 200 {
        format!("{clipped}…")
    } else {
        clipped
    }
}

/// 列出该服务提供的模型名。OpenAI 兼容服务在 `data[].id`，少数在 `models[].name`。
pub fn list_models(base_url: &str, key: &str) -> Result<Vec<String>> {
    let base = normalize_chat_base(base_url);
    if base.is_empty() {
        return Err("先填服务地址".into());
    }
    https_or_loopback(&base)?;
    let url = format!("{base}/models");
    crate::log::note("连接", format!("获取模型列表：{url}"));
    let mut request = client()?.get(&url);
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send().map_err(|error| match error.is_timeout() {
        true => "获取模型列表超时".to_owned(),
        false => "无法连接该地址".to_owned(),
    })?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(explain(status, &text, key));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|_| "返回的不是 JSON".to_string())?;
    let mut names: Vec<String> = Vec::new();
    for row in parsed["data"].as_array().cloned().unwrap_or_default() {
        if let Some(id) = row["id"].as_str() {
            names.push(id.to_owned());
        }
    }
    if names.is_empty() {
        for row in parsed["models"].as_array().cloned().unwrap_or_default() {
            if let Some(name) = row["name"].as_str().or_else(|| row["id"].as_str()) {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    if names.is_empty() {
        return Err("该地址没有返回模型列表；可能不支持 /models，手填模型名即可".into());
    }
    crate::log::note("连接", format!("拿到 {} 个模型", names.len()));
    Ok(names)
}

/// 发一条最短的探针，用返回向量的长度作为维度。
///
/// 这比猜准：同一系列模型的维度并不统一（有的 1024，有的 4096），
/// 填错只会在建索引时失败，所以直接问一次最省事。
pub fn embedding_dimensions(endpoint: &str, model: &str, key: &str) -> Result<usize> {
    let url = normalize_embedding_endpoint(endpoint);
    if url.is_empty() || model.trim().is_empty() {
        return Err("先填服务地址与模型名".into());
    }
    https_or_loopback(&url)?;
    if key.is_empty() {
        return Err("该服务需要密钥；先填密钥".into());
    }
    crate::log::note("连接", format!("探测编码维度：{model} @ {url}"));
    let started = std::time::Instant::now();
    let response = client()?
        .post(&url)
        .bearer_auth(key)
        .json(&json!({"model": model, "input": ["维度探测"], "encoding_format": "float"}))
        .send()
        .map_err(|error| match error.is_timeout() {
            true => "探测超时".to_owned(),
            false => "无法连接该地址".to_owned(),
        })?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        crate::log::done("连接", format!("探测失败 {status}"), started);
        return Err(explain(status, &text, key));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|_| "返回的不是 JSON".to_string())?;
    let dimensions = parsed["data"][0]["embedding"]
        .as_array()
        .map(|values| values.len())
        .ok_or("返回里没有向量；检查地址是否为编码接口")?;
    if dimensions == 0 {
        return Err("返回的向量是空的".into());
    }
    crate::log::done("连接", format!("维度 {dimensions}"), started);
    Ok(dimensions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_base_is_completed_from_a_bare_host() {
        assert_eq!(
            normalize_chat_base("api.deepseek.com"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            normalize_chat_base("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1"
        );
        // 已经规范过的地址不该被再改一次
        assert_eq!(
            normalize_chat_base(&normalize_chat_base("example.com/v1")),
            "https://example.com/v1"
        );
        assert_eq!(normalize_chat_base("  "), "");
    }

    #[test]
    fn embedding_endpoint_always_ends_with_embeddings() {
        assert_eq!(
            normalize_embedding_endpoint("emrank.tech"),
            "https://emrank.tech/v1/embeddings"
        );
        assert_eq!(
            normalize_embedding_endpoint("https://emrank.tech/v1/embeddings"),
            "https://emrank.tech/v1/embeddings"
        );
        assert_eq!(
            normalize_embedding_endpoint("https://emrank.tech/v1"),
            "https://emrank.tech/v1/embeddings"
        );
        assert_eq!(
            normalize_embedding_endpoint("http://127.0.0.1:8899/v1"),
            "http://127.0.0.1:8899/v1/embeddings"
        );
    }
}

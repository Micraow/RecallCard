//! OpenAI 兼容的对话补全客户端。
//!
//! 只接受 https 或本机地址；密钥只进请求头，不写日志、不进错误文本。
//! 用途仅限用户明确点击的那一次整理，不做后台请求。

use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

use crate::model::Result;
use crate::settings::LlmSettings;

const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const TIMEOUT_SECONDS: u64 = 180;

/// 请求一次补全，返回模型输出的纯文本。
pub fn complete(settings: &LlmSettings, system: &str, user: &str) -> Result<String> {
    if settings.base_url.is_empty() || settings.model.is_empty() {
        return Err("尚未配置模型服务；请到设置页填写地址与模型名".into());
    }
    if settings.key.is_empty() {
        return Err("尚未配置模型服务密钥；请到设置页填写".into());
    }
    let url = format!(
        "{}/chat/completions",
        settings.base_url.trim_end_matches('/')
    );
    let max_tokens = if settings.max_output_tokens == 0 {
        4096
    } else {
        settings.max_output_tokens
    };
    let body = json!({
        "model": settings.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": max_tokens,
        "temperature": 0.2,
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECONDS))
        .build()
        .map_err(|_| "无法初始化网络客户端".to_owned())?;
    crate::log::note(
        "模型",
        format!(
            "请求 {} · 输入 {} 字符 · 上限 {} tokens",
            settings.model,
            user.chars().count(),
            max_tokens
        ),
    );
    let started = std::time::Instant::now();
    let response = client
        .post(&url)
        .bearer_auth(&settings.key)
        .json(&body)
        .send()
        .map_err(|error| match error.is_timeout() {
            true => "模型服务响应超时".to_owned(),
            false => "无法连接模型服务；请检查地址与网络".to_owned(),
        })?;
    let status = response.status();
    crate::log::done("模型", format!("返回 {status}"), started);
    let mut buffer = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut buffer)
        .map_err(|_| "读取模型服务响应失败".to_owned())?;
    let text = String::from_utf8_lossy(&buffer).into_owned();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "模型服务拒绝了凭据；请检查密钥".to_owned(),
            404 => "模型服务地址或模型名不正确".to_owned(),
            429 => "模型服务限流，请稍后再试".to_owned(),
            code => format!("模型服务返回错误（HTTP {code}）：{}", short(&text)),
        });
    }
    let parsed: Value =
        serde_json::from_str(&text).map_err(|_| "模型服务返回的不是 JSON".to_owned())?;
    let content = parsed["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("模型服务没有返回文本内容")?;
    Ok(content.to_owned())
}

/// 错误文本里只带一小段响应，避免把整页报文塞进界面。
fn short(text: &str) -> String {
    let trimmed = text.trim();
    let clipped: String = trimmed.chars().take(200).collect();
    if trimmed.chars().count() > 200 {
        format!("{clipped}…")
    } else {
        clipped
    }
}

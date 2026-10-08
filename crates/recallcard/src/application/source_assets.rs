//! 保存导出真实提供的引用/文件元数据；没有负载的工具节点保留明确缺口。
use super::{
    import_reader::{ImportedConversation, ReadReport},
    AppError, AppResult, ErrorCode,
};
use crate::{EventInput, Origin, Role};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::InvalidConversation,
        "来源引用或文件元数据格式无效",
        "重新取得完整官方导出，不会猜测缺失字段",
    )
}
fn text(value: &Value, max: usize) -> AppResult<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.contains('\0'))
        .ok_or_else(invalid)
}
fn optional_title(value: &Value) -> AppResult<&str> {
    value
        .as_str()
        .filter(|s| s.len() <= 16384 && !s.contains('\0'))
        .ok_or_else(invalid)
}

pub(super) fn deepseek(
    value: &Value,
    conversation: &mut ImportedConversation,
    report: &mut ReadReport,
    scope: &str,
) -> AppResult<()> {
    let mapping = value["mapping"].as_object().ok_or_else(invalid)?;
    let mut by_id: BTreeMap<String, EventInput> = std::mem::take(&mut conversation.events)
        .into_iter()
        .map(|e| (e.source.message_id.clone(), e))
        .collect();
    let mut ready: BTreeSet<&str> = mapping
        .iter()
        .filter_map(|(id, n)| {
            n.get("parent")
                .is_none_or(Value::is_null)
                .then_some(id.as_str())
        })
        .collect();
    let mut order = Vec::with_capacity(mapping.len());
    while let Some(id) = ready.pop_first() {
        order.push(id);
        for child in mapping[id]["children"].as_array().ok_or_else(invalid)? {
            ready.insert(child.as_str().ok_or_else(invalid)?);
        }
    }
    // 基础适配器已验证图结构；仍不允许辅助资产恢复逻辑改变图覆盖范围。
    if order.len() != mapping.len() {
        return Err(invalid());
    }
    let mut budget = crate::import::NormalizedBudget::default();
    for id in order.iter().copied() {
        let node = &mapping[id];
        let message = &node["message"];
        if message.is_null() {
            continue;
        }
        let mut files = Vec::new();
        let mut citations = Vec::new();
        let mut traces = Vec::new();
        let mut unsupported = Vec::new();
        for (index, fragment) in message["fragments"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .enumerate()
        {
            if matches!(
                fragment["type"].as_str(),
                Some("FILE" | "SEARCH" | "TOOL_SEARCH" | "TOOL_OPEN" | "TOOL_FIND")
            ) {
                conversation.deepseek_coverage.unsupported_fragments_skipped = conversation
                    .deepseek_coverage
                    .unsupported_fragments_skipped
                    .saturating_sub(1);
            }
            match fragment["type"].as_str() {
                Some("FILE") => {
                    for file in fragment["files"].as_array().ok_or_else(invalid)? {
                        files.push(json!({"source_file_id":text(&file["file_id"],2048)?,"name":text(&file["file_name"],4096)?,"byte_count":file["file_size"].as_u64().ok_or_else(invalid)?,"payload_status":"not_in_export","fragment_index":index}));
                    }
                }
                Some(kind @ ("SEARCH" | "TOOL_SEARCH")) => {
                    let results = match fragment.get("results") {
                        None | Some(Value::Null) => {
                            traces.push(json!({"type":kind,"fragment_index":index,"payload_status":"not_in_export"}));
                            report.trace_placeholders += 1;
                            continue;
                        }
                        Some(value) => value.as_array().ok_or_else(invalid)?,
                    };
                    for result in results {
                        citations.push(json!({"url":text(&result["url"],16384)?,"title":optional_title(&result["title"])?,"fragment_type":kind,"fragment_index":index}));
                    }
                    traces.push(json!({"type":kind,"fragment_index":index,"payload_status":"references_only","reference_count":results.len()}));
                }
                Some(kind @ ("TOOL_OPEN" | "TOOL_FIND")) => {
                    traces.push(json!({"type":kind,"fragment_index":index,"payload_status":"not_in_export"}));
                    report.trace_placeholders += 1;
                }
                Some("REQUEST" | "RESPONSE" | "TEMPLATE_RESPONSE" | "THINK") => {}
                Some(kind) => {
                    unsupported.push(
                        json!({"type":kind,"fragment_index":index,"payload_status":"unsupported"}),
                    );
                }
                None => {}
            }
        }
        let assets = !files.is_empty()
            || !citations.is_empty()
            || !traces.is_empty()
            || !unsupported.is_empty();
        report.file_references += files.len() as u64;
        report.citations += citations.len() as u64;
        let mut event = if let Some(event) = by_id.remove(id) {
            event
        } else if assets {
            let role = if !files.is_empty() && traces.is_empty() {
                Role::User
            } else {
                Role::Tool
            };
            let origin = if role == Role::User {
                Origin::ExternalQuote
            } else {
                Origin::ToolOutput
            };
            let event:EventInput=serde_json::from_value(json!({
                "occurred_at":message["inserted_at"],"scope":scope,"role":role,"origin":origin,"kind":if role==Role::User{"file"}else{"tool_result"},"content":"",
                "source":{"platform":"deepseek","conversation_id":conversation.source_id,"message_id":id},
                "capture":{"completeness":"partial","reason":"exported_source_metadata_without_message_body"},
                "metadata":{"import_adapter":"deepseek-export","conversation_title":conversation.title,"branch":"all_exported_nodes","previous_message_id":node["parent"],"deepseek":{"node_id":id,"parent_id":node["parent"],"children_ids":node["children"],"body_present":false,"conversation_has_branches":conversation.deepseek_coverage.branch_points>0}}
            })).map_err(|_|invalid())?;
            // 从旧覆盖的 omitted 项转为真实可引用资产事件，不把它标成正文消息。
            if conversation.deepseek_coverage.unsupported_messages_skipped > 0 {
                conversation.deepseek_coverage.unsupported_messages_skipped -= 1;
            }
            event
        } else {
            continue;
        };
        if assets {
            event.metadata["source_assets"] = json!({"schema":"recallcard.source-assets/1","files":files,"citations":citations,"tool_trace":traces,"unsupported_fragments":unsupported});
        }
        event
            .validate()
            .map_err(|e| super::import_reader::adapter_error(e, "DeepSeek"))?;
        budget.add(&event).map_err(|message| {
            AppError::new(
                ErrorCode::ResourceLimit,
                message,
                "查看受影响会话；不会截断内容",
            )
        })?;
        conversation.events.push(event);
    }
    if !by_id.is_empty() {
        return Err(invalid());
    }
    let saved: BTreeSet<&str> = conversation
        .events
        .iter()
        .map(|e| e.source.message_id.as_str())
        .collect();
    let mut nearest: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    let mut omitted: BTreeMap<&str, u64> = BTreeMap::new();
    for id in order {
        let parent = mapping[id]["parent"].as_str();
        let visible = if saved.contains(id) {
            Some(id)
        } else {
            parent.and_then(|p| nearest[p])
        };
        let gap = if saved.contains(id) || (parent.is_none() && mapping[id]["message"].is_null()) {
            0
        } else {
            parent.map(|p| omitted[p]).unwrap_or(0) + 1
        };
        nearest.insert(id, visible);
        omitted.insert(id, gap);
    }
    let annotations: Vec<_> = conversation
        .events
        .iter()
        .map(|e| {
            let parent = mapping[&e.source.message_id]["parent"].as_str();
            (
                parent.and_then(|p| nearest[p]).map(str::to_owned),
                parent.map(|p| omitted[p]).unwrap_or(0),
                parent.is_some_and(|p| !mapping[p]["message"].is_null() && !saved.contains(p)),
            )
        })
        .collect();
    for (event, (parent, gap, missing)) in conversation.events.iter_mut().zip(annotations) {
        event.metadata["deepseek"]["nearest_visible_parent_id"] = json!(parent);
        event.metadata["deepseek"]["omitted_parent_nodes"] = json!(gap);
        event.metadata["deepseek"]["parent_message_omitted"] = json!(missing);
    }
    Ok(())
}

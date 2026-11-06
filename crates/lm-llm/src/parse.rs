//! LLM が返した JSON を `RawMinutes`（未検証）にする。形が合わない出力はエラーにする。
//! 根拠の実在や本文の無害化は、このあとの `lm_ingest::minutes::validate` が行う。

use lm_ingest::{IngestError, RawItem, RawMinutes};
use serde_json::Value;

/// 受け付ける出力の最大バイト数（解析前の上限）。
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

fn bad(what: &str) -> IngestError {
    IngestError::Extract(format!("LLM の出力を読めません: {what}"))
}

/// 空文字は「なし」。
fn optional(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn item(v: &Value) -> Result<RawItem, IngestError> {
    let text = v
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("text がありません"))?;
    let cited = v
        .get("cited")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("cited がありません"))?;
    Ok(RawItem {
        text: text.to_string(),
        assignee: v.get("assignee").and_then(optional),
        due: v.get("due").and_then(optional),
        // 数でないもの・負の数・大きすぎる数は、引用として数えない（検証で「根拠なし」になる）。
        cited_start_ms: cited.iter().filter_map(Value::as_u64).collect(),
    })
}

fn section(root: &Value, key: &str) -> Result<Vec<RawItem>, IngestError> {
    root.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| bad(&format!("{key} がありません")))?
        .iter()
        .map(item)
        .collect()
}

/// 出力の JSON を読む。前後の空白は許す。
pub fn parse_minutes(output: &str) -> Result<RawMinutes, IngestError> {
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(bad("出力が大きすぎます"));
    }
    let root: Value = serde_json::from_str(output.trim())
        .map_err(|_| bad("JSON ではありません（出力が途中で切れた可能性があります）"))?;
    Ok(RawMinutes {
        summary: section(&root, "summary")?,
        decisions: section(&root, "decisions")?,
        action_items: section(&root, "action_items")?,
        open_issues: section(&root, "open_issues")?,
    })
}

#[cfg(test)]
mod tests;

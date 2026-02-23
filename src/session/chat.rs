use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::session::model::{SessionMeta, SessionStatus, ToolKind};

const MAX_CHAT_MESSAGES: usize = 400;
const MAX_TEXT_LEN: usize = 6000;

#[derive(Debug, Clone, Serialize)]
pub struct ChatSnapshot {
    pub available: bool,
    pub transcript_path: Option<String>,
    pub permission_mode: String,
    pub view_mode: String,
    pub state: String,
    pub status_label: String,
    pub messages: Vec<ChatMessage>,
    pub pending_question: Option<PendingQuestion>,
    pub plan: Option<PlanSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub kind: String,
    pub text: String,
    pub timestamp: Option<String>,
    pub tool_name: Option<String>,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingQuestion {
    pub tool_use_id: String,
    pub questions: Vec<PendingQuestionItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingQuestionItem {
    pub header: String,
    pub question: String,
    pub options: Vec<PendingQuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingQuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSummary {
    pub source: String,
    pub items: Vec<String>,
    pub markdown: Option<String>,
}

pub fn expected_transcript_path(meta: &SessionMeta) -> Option<PathBuf> {
    if meta.tool != ToolKind::Claude {
        return None;
    }

    let home = dirs::home_dir()?;
    let project_key = claude_project_key(&meta.working_dir);
    Some(
        home.join(".claude")
            .join("projects")
            .join(project_key)
            .join(format!("{}.jsonl", meta.id)),
    )
}

pub fn fallback_transcript_path(meta: &SessionMeta) -> Option<PathBuf> {
    if meta.tool != ToolKind::Claude {
        return None;
    }

    let home = dirs::home_dir()?;
    let projects_dir = home.join(".claude").join("projects");
    let file_name = format!("{}.jsonl", meta.id);

    let entries = std::fs::read_dir(projects_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path().join(&file_name);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

pub fn parse_snapshot(
    meta: &SessionMeta,
    transcript_path: Option<&Path>,
    content: Option<&str>,
) -> ChatSnapshot {
    let mut parser = ChatParser {
        permission_mode: "default".to_string(),
        view_mode: "default".to_string(),
        messages: Vec::new(),
        pending_question: None,
        plan: None,
        progress_hint: None,
        last_event_type: None,
    };

    if let Some(text) = content {
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            parser.consume(value);
        }
    }

    let (state, status_label) = derive_state(meta, &parser);

    ChatSnapshot {
        available: content.is_some(),
        transcript_path: transcript_path.map(|p| p.display().to_string()),
        permission_mode: parser.permission_mode,
        view_mode: parser.view_mode,
        state,
        status_label,
        messages: parser.messages,
        pending_question: parser.pending_question,
        plan: parser.plan,
    }
}

fn derive_state(meta: &SessionMeta, parser: &ChatParser) -> (String, String) {
    if meta.status != SessionStatus::Running {
        return ("stopped".to_string(), format!("Session {}", meta.status));
    }

    if parser.pending_question.is_some() {
        return (
            "awaiting_input".to_string(),
            "Waiting for your answer".to_string(),
        );
    }

    if let Some(last_type) = &parser.last_event_type
        && last_type == "progress"
        && let Some(hint) = &parser.progress_hint
    {
        return ("working".to_string(), hint.clone());
    }

    if let Some(last) = parser.messages.last() {
        if last.role == "user" {
            return ("thinking".to_string(), "Claude is thinking".to_string());
        }
        if last.kind == "thinking" {
            return ("thinking".to_string(), "Claude is reasoning".to_string());
        }
    }

    ("idle".to_string(), "Ready".to_string())
}

struct ChatParser {
    permission_mode: String,
    view_mode: String,
    messages: Vec<ChatMessage>,
    pending_question: Option<PendingQuestion>,
    plan: Option<PlanSummary>,
    progress_hint: Option<String>,
    last_event_type: Option<String>,
}

impl ChatParser {
    fn consume(&mut self, event: Value) {
        if let Some(plan_text) = event.get("planContent").and_then(Value::as_str) {
            self.capture_plan("planContent", plan_text);
        }

        if let Some(mode) = event.get("permissionMode").and_then(Value::as_str) {
            self.permission_mode = mode.to_string();
            self.view_mode = normalize_view_mode(mode);
        }

        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let timestamp = event
            .get("timestamp")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        match event_type.as_str() {
            "assistant" => self.consume_assistant(&event, timestamp),
            "user" => self.consume_user(&event, timestamp),
            "progress" => self.consume_progress(&event),
            "system" => self.consume_system(&event),
            _ => {}
        }

        self.last_event_type = Some(event_type);
        if self.messages.len() > MAX_CHAT_MESSAGES {
            let extra = self.messages.len() - MAX_CHAT_MESSAGES;
            self.messages.drain(0..extra);
        }
    }

    fn consume_assistant(&mut self, event: &Value, timestamp: Option<String>) {
        self.progress_hint = None;

        let Some(message) = event.get("message") else {
            return;
        };

        let Some(content) = message.get("content").and_then(Value::as_array) else {
            return;
        };

        for block in content {
            let block_type = block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match block_type {
                "text" => {
                    let text = compact_text(block.get("text"));
                    if !text.is_empty() {
                        self.push_message(
                            event
                                .get("uuid")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            "assistant",
                            "text",
                            text.clone(),
                            timestamp.clone(),
                            None,
                            false,
                        );
                        self.capture_plan_from_text(&text);
                    }
                }
                "thinking" => {
                    let text = compact_text(block.get("thinking"));
                    if !text.is_empty() {
                        self.push_message(
                            event
                                .get("uuid")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            "assistant",
                            "thinking",
                            text,
                            timestamp.clone(),
                            None,
                            false,
                        );
                    }
                }
                "tool_use" => {
                    let tool_name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let tool_use_id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();

                    if tool_name == "AskUserQuestion" {
                        if let Some(question) = parse_pending_question(block, &tool_use_id) {
                            self.pending_question = Some(question.clone());
                            self.push_message(
                                event
                                    .get("uuid")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                "assistant",
                                "ask_user_question",
                                "Claude is asking for your input".to_string(),
                                timestamp.clone(),
                                Some(tool_name.clone()),
                                false,
                            );
                        }
                    } else if tool_name == "EnterPlanMode" {
                        self.permission_mode = "plan".to_string();
                        self.view_mode = "plan".to_string();
                    } else if tool_name == "ExitPlanMode" {
                        self.view_mode = normalize_view_mode(&self.permission_mode);
                        if let Some(plan_text) = block
                            .get("input")
                            .and_then(|v| v.get("plan"))
                            .and_then(Value::as_str)
                        {
                            self.capture_plan("ExitPlanMode", plan_text);
                        }
                    }

                    self.push_message(
                        event
                            .get("uuid")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        "tool",
                        "tool_use",
                        format!("Using {tool_name}"),
                        timestamp.clone(),
                        Some(tool_name),
                        false,
                    );
                }
                _ => {}
            }
        }
    }

    fn consume_user(&mut self, event: &Value, timestamp: Option<String>) {
        self.progress_hint = None;

        let Some(message) = event.get("message") else {
            return;
        };

        let content = message.get("content");

        if let Some(text) = content.and_then(Value::as_str) {
            self.consume_user_text(event, timestamp, text);
            return;
        }

        let Some(items) = content.and_then(Value::as_array) else {
            return;
        };

        for item in items {
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
            match item_type {
                "text" => {
                    self.consume_user_text(
                        event,
                        timestamp.clone(),
                        item.get("text").and_then(Value::as_str).unwrap_or_default(),
                    );
                }
                "tool_result" => {
                    let text = compact_text(item.get("content"));
                    let is_error = item
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);

                    if let Some(tool_use_id) = item.get("tool_use_id").and_then(Value::as_str)
                        && let Some(pending) = &self.pending_question
                        && pending.tool_use_id == tool_use_id
                    {
                        self.pending_question = None;
                    }

                    self.push_message(
                        event
                            .get("uuid")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        "tool",
                        "tool_result",
                        if text.is_empty() {
                            if is_error {
                                "Tool call was rejected".to_string()
                            } else {
                                "Tool call completed".to_string()
                            }
                        } else {
                            text
                        },
                        timestamp.clone(),
                        None,
                        is_error,
                    );
                }
                _ => {}
            }
        }

        self.capture_plan_from_tool_use_result(event);
    }

    fn consume_system(&mut self, event: &Value) {
        if let Some(mode) = event.get("permissionMode").and_then(Value::as_str) {
            self.permission_mode = mode.to_string();
            self.view_mode = normalize_view_mode(mode);
        }
    }

    fn consume_progress(&mut self, event: &Value) {
        let Some(data) = event.get("data") else {
            return;
        };

        if data.get("type").and_then(Value::as_str) != Some("agent_progress") {
            return;
        }

        let tool_name = data
            .get("message")
            .and_then(|m| m.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|first| {
                if first.get("type").and_then(Value::as_str) == Some("tool_use") {
                    first.get("name").and_then(Value::as_str)
                } else {
                    None
                }
            });

        if let Some(name) = tool_name {
            self.progress_hint = Some(format!("Running {name}"));
        }
    }

    fn consume_user_text(&mut self, event: &Value, timestamp: Option<String>, text: &str) {
        let raw = text.trim();
        if raw.is_empty() {
            return;
        }

        if self.consume_local_command_markup(event, raw, timestamp.clone()) {
            return;
        }

        if let Some(pending) = &self.pending_question
            && pending.tool_use_id.starts_with("localcmd-")
        {
            self.pending_question = None;
        }

        let compacted = compact_string(raw);
        if compacted.is_empty() {
            return;
        }

        self.push_message(
            event
                .get("uuid")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "user",
            "text",
            compacted,
            timestamp,
            None,
            false,
        );
    }

    fn consume_local_command_markup(
        &mut self,
        event: &Value,
        text: &str,
        timestamp: Option<String>,
    ) -> bool {
        if extract_tag_content(text, "local-command-caveat").is_some() {
            return true;
        }

        if extract_tag_content(text, "command-name").is_some() {
            return true;
        }

        let Some(stdout) = extract_tag_content(text, "local-command-stdout") else {
            return false;
        };

        let cleaned = compact_string(strip_ansi(stdout).trim());
        if cleaned.is_empty() {
            return true;
        }

        if cleaned.eq_ignore_ascii_case("enabled plan mode") {
            self.permission_mode = "plan".to_string();
            self.view_mode = "plan".to_string();
        } else if cleaned.eq_ignore_ascii_case("disabled plan mode") {
            self.permission_mode = "default".to_string();
            self.view_mode = "default".to_string();
        }

        if let Some(pending) = parse_local_command_options(
            &cleaned,
            event
                .get("uuid")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) {
            self.pending_question = Some(pending);
        }

        self.push_message(
            event
                .get("uuid")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "system",
            "local_command",
            cleaned,
            timestamp,
            None,
            false,
        );
        true
    }

    fn capture_plan_from_text(&mut self, text: &str) {
        let items = extract_plan_items(text);
        if items.len() >= 2 {
            self.plan = Some(PlanSummary {
                source: "assistant_text".to_string(),
                items,
                markdown: Some(compact_string(text)),
            });
        }
    }

    fn capture_plan(&mut self, source: &str, text: &str) {
        let mut items = extract_plan_items(text);
        if items.is_empty() {
            items.push(compact_string(text));
        }
        self.plan = Some(PlanSummary {
            source: source.to_string(),
            items,
            markdown: Some(compact_string(text)),
        });
    }

    fn capture_plan_from_tool_use_result(&mut self, event: &Value) {
        let Some(tool_result) = event.get("toolUseResult") else {
            return;
        };
        let Some(file_path) = tool_result.get("filePath").and_then(Value::as_str) else {
            return;
        };
        if !file_path.ends_with(".md") || !file_path.contains("/.claude/plans/") {
            return;
        }

        if let Some(content) = tool_result.get("content").and_then(Value::as_str) {
            self.capture_plan("toolUseResult", content);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_message(
        &mut self,
        id: &str,
        role: &str,
        kind: &str,
        text: String,
        timestamp: Option<String>,
        tool_name: Option<String>,
        is_error: bool,
    ) {
        self.messages.push(ChatMessage {
            id: if id.is_empty() {
                format!("msg-{}", self.messages.len() + 1)
            } else {
                id.to_string()
            },
            role: role.to_string(),
            kind: kind.to_string(),
            text,
            timestamp,
            tool_name,
            is_error,
        });
    }
}

pub fn augment_snapshot_from_terminal_output(
    mut snapshot: ChatSnapshot,
    terminal_output: &str,
) -> ChatSnapshot {
    if snapshot.pending_question.is_some() {
        return snapshot;
    }

    let normalized = normalize_terminal_output(terminal_output);
    if normalized.is_empty() {
        return snapshot;
    }

    if let Some(question) = parse_terminal_choice_prompt(&normalized) {
        snapshot.pending_question = Some(question);
    }

    snapshot
}

fn parse_pending_question(block: &Value, tool_use_id: &str) -> Option<PendingQuestion> {
    let questions = block
        .get("input")
        .and_then(|v| v.get("questions"))
        .and_then(Value::as_array)?;

    let mut items = Vec::new();
    for q in questions {
        let options = q
            .get("options")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|opt| PendingQuestionOption {
                        label: opt
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        description: opt
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        items.push(PendingQuestionItem {
            header: q
                .get("header")
                .and_then(Value::as_str)
                .unwrap_or("Question")
                .to_string(),
            question: q
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            options,
            multi_select: q
                .get("multiSelect")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }

    if items.is_empty() {
        return None;
    }

    Some(PendingQuestion {
        tool_use_id: tool_use_id.to_string(),
        questions: items,
    })
}

fn parse_local_command_options(text: &str, id_hint: &str) -> Option<PendingQuestion> {
    let mut prompt_line = None;
    let mut options = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(option) = parse_numbered_option(trimmed) {
            options.push(PendingQuestionOption {
                label: option,
                description: String::new(),
            });
            continue;
        }

        if prompt_line.is_none() {
            prompt_line = Some(compact_string(trimmed));
        }
    }

    if options.len() < 2 {
        return None;
    }

    let prompt = prompt_line.unwrap_or_else(|| "Select an option".to_string());
    let prompt_lower = prompt.to_lowercase();
    let looks_like_prompt = prompt.contains('?')
        || prompt_lower.contains("choose")
        || prompt_lower.contains("select")
        || prompt_lower.contains("option");

    if !looks_like_prompt {
        return None;
    }

    Some(PendingQuestion {
        tool_use_id: format!("localcmd-{id_hint}"),
        questions: vec![PendingQuestionItem {
            header: "Choose".to_string(),
            question: prompt,
            options,
            multi_select: false,
        }],
    })
}

fn parse_numbered_option(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let mut idx = 0usize;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            idx += ch.len_utf8();
            continue;
        }
        break;
    }

    if idx == 0 {
        return None;
    }

    let rest = trimmed[idx..].trim_start();
    let rest = if let Some(v) = rest.strip_prefix('.') {
        v
    } else if let Some(v) = rest.strip_prefix(')') {
        v
    } else {
        return None;
    };

    let label = compact_string(rest.trim());
    if label.is_empty() { None } else { Some(label) }
}

fn parse_terminal_choice_prompt(output: &str) -> Option<PendingQuestion> {
    let tail = tail_chars(output, 25_000);
    let lines = tail
        .lines()
        .map(compact_string)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let mut start_idx = None;
    for idx in (0..lines.len()).rev() {
        if let Some((num, _)) = parse_numbered_menu_line(&lines[idx])
            && num == 1
        {
            start_idx = Some(idx);
            break;
        }
    }
    let start_idx = start_idx?;

    let mut options = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let end = usize::min(lines.len(), start_idx + 12);
    for line in lines.iter().take(end).skip(start_idx) {
        if let Some((num, label)) = parse_numbered_menu_line(line)
            && seen.insert(num)
        {
            options.push(PendingQuestionOption {
                label,
                description: String::new(),
            });
        }
    }
    if options.len() < 2 {
        return None;
    }

    let search_start = start_idx.saturating_sub(8);
    let mut question_parts = Vec::new();
    for line in lines.iter().take(start_idx + 1).skip(search_start) {
        let lower = line.to_lowercase();
        if line.contains('?')
            || lower.contains("would you like to")
            || lower.contains("ready to execute")
        {
            question_parts.push(line.clone());
        }
    }

    if question_parts.is_empty() {
        return None;
    }
    let question = compact_string(&question_parts.join(" "));

    Some(PendingQuestion {
        tool_use_id: "terminal-choice".to_string(),
        questions: vec![PendingQuestionItem {
            header: "Continue".to_string(),
            question,
            options,
            multi_select: false,
        }],
    })
}

fn parse_numbered_menu_line(line: &str) -> Option<(usize, String)> {
    let trimmed = line
        .trim_start()
        .trim_start_matches(['❯', '>', '›', '•'])
        .trim_start();

    let mut idx = 0usize;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            idx += ch.len_utf8();
            continue;
        }
        break;
    }
    if idx == 0 {
        return None;
    }

    let num = trimmed[..idx].parse::<usize>().ok()?;
    let rest = trimmed[idx..].trim_start();
    let rest = rest.strip_prefix('.')?;
    let label = compact_string(rest.trim());
    if label.is_empty() {
        None
    } else {
        Some((num, label))
    }
}

fn extract_plan_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.len() < 2 {
            continue;
        }
        if let Some(stripped) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            items.push(compact_string(stripped));
            continue;
        }

        if let Some(dot_idx) = trimmed.find('.')
            && dot_idx > 0
            && dot_idx < 3
            && trimmed[..dot_idx].chars().all(|c| c.is_ascii_digit())
        {
            let rest = trimmed[dot_idx + 1..].trim();
            if !rest.is_empty() {
                items.push(compact_string(rest));
            }
        }
    }
    items
}

fn extract_tag_content<'a>(input: &'a str, tag: &str) -> Option<&'a str> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let start_idx = input.find(&start)?;
    let content_start = start_idx + start.len();
    let end_idx = input[content_start..].find(&end)?;
    Some(&input[content_start..content_start + end_idx])
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('[') => {
                let _ = chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                let _ = chars.next();
                loop {
                    match chars.next() {
                        Some('\u{7}') | None => break,
                        Some('\u{1b}') => {
                            if matches!(chars.peek(), Some('\\')) {
                                let _ = chars.next();
                                break;
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
            _ => {}
        }
    }

    out
}

fn normalize_terminal_output(input: &str) -> String {
    strip_ansi(input)
        .replace('\u{7}', "")
        .replace('\r', "\n")
        .replace('\u{0008}', "")
}

fn tail_chars(input: &str, max_chars: usize) -> String {
    let count = input.chars().count();
    if count <= max_chars {
        return input.to_string();
    }
    input.chars().skip(count - max_chars).collect::<String>()
}

fn compact_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => compact_string(s),
        Some(v) => compact_string(&v.to_string()),
        None => String::new(),
    }
}

fn compact_string(input: &str) -> String {
    let mut s = input.trim().to_string();
    if s.len() > MAX_TEXT_LEN {
        s.truncate(MAX_TEXT_LEN);
        s.push('…');
    }
    s
}

fn normalize_view_mode(permission_mode: &str) -> String {
    match permission_mode {
        "plan" => "plan".to_string(),
        "bypassPermissions" | "acceptEdits" => "yolo".to_string(),
        _ => "default".to_string(),
    }
}

fn claude_project_key(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn meta() -> SessionMeta {
        SessionMeta {
            id: Uuid::nil(),
            name: "test".to_string(),
            tool: ToolKind::Claude,
            status: SessionStatus::Running,
            working_dir: PathBuf::from("/tmp"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            pid: None,
            extra_args: vec![],
        }
    }

    #[test]
    fn parses_basic_chat_and_plan() {
        let transcript = r#"{"type":"user","timestamp":"2026-02-22T18:00:00.000Z","message":{"role":"user","content":"Make a plan"},"permissionMode":"default","uuid":"u1"}
{"type":"assistant","timestamp":"2026-02-22T18:00:01.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Plan:\\n- Read code\\n- Edit files"}]},"uuid":"a1"}"#;

        let snapshot = parse_snapshot(&meta(), None, Some(transcript));
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.view_mode, "default");
        assert_eq!(snapshot.state, "idle");
    }

    #[test]
    fn parses_pending_ask_user_question() {
        let transcript = r#"{"type":"assistant","timestamp":"2026-02-22T18:00:01.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-1","name":"AskUserQuestion","input":{"questions":[{"header":"Confirm","question":"Proceed?","options":[{"label":"Yes","description":"Go"},{"label":"No","description":"Stop"}],"multiSelect":false}]}}]},"uuid":"a1"}"#;

        let snapshot = parse_snapshot(&meta(), None, Some(transcript));
        assert_eq!(snapshot.state, "awaiting_input");
        assert!(snapshot.pending_question.is_some());
        let pending = snapshot.pending_question.expect("question should exist");
        assert_eq!(pending.questions.len(), 1);
        assert_eq!(pending.questions[0].options.len(), 2);
    }

    #[test]
    fn extracts_plan_items_from_text() {
        let items = extract_plan_items("Plan:\n1. Read code\n2. Edit files");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "Read code");
        assert_eq!(items[1], "Edit files");
    }

    #[test]
    fn parses_plan_content_field() {
        let transcript = r##"{"type":"user","timestamp":"2026-02-22T18:00:00.000Z","message":{"role":"user","content":"Implement this"},"planContent":"# Plan\n1. Read code\n2. Edit files","uuid":"u1"}"##;

        let snapshot = parse_snapshot(&meta(), None, Some(transcript));
        assert!(snapshot.plan.is_some());
        let plan = snapshot.plan.expect("plan should exist");
        assert_eq!(plan.source, "planContent");
        assert_eq!(plan.items.len(), 2);
    }

    #[test]
    fn parses_local_plan_mode_commands() {
        let transcript = r#"{"type":"user","timestamp":"2026-02-22T18:00:00.000Z","message":{"role":"user","content":"<local-command-caveat>Caveat</local-command-caveat>"},"uuid":"u1"}
{"type":"user","timestamp":"2026-02-22T18:00:01.000Z","message":{"role":"user","content":"<command-name>/plan</command-name>"},"uuid":"u2"}
{"type":"user","timestamp":"2026-02-22T18:00:02.000Z","message":{"role":"user","content":"<local-command-stdout>Enabled plan mode</local-command-stdout>"},"uuid":"u3"}"#;

        let snapshot = parse_snapshot(&meta(), None, Some(transcript));
        assert_eq!(snapshot.permission_mode, "plan");
        assert_eq!(snapshot.view_mode, "plan");
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].role, "system");
        assert_eq!(snapshot.messages[0].text, "Enabled plan mode");
    }

    #[test]
    fn parses_local_command_options_into_pending_question() {
        let transcript = r#"{"type":"user","timestamp":"2026-02-22T18:00:01.000Z","message":{"role":"user","content":"<local-command-stdout>Choose an option:\n1. Continue\n2. Stop</local-command-stdout>"},"uuid":"u1"}"#;

        let snapshot = parse_snapshot(&meta(), None, Some(transcript));
        assert_eq!(snapshot.state, "awaiting_input");
        assert!(snapshot.pending_question.is_some());
        let pending = snapshot.pending_question.expect("question should exist");
        assert_eq!(pending.questions[0].options.len(), 2);
        assert_eq!(pending.questions[0].options[0].label, "Continue");
    }

    #[test]
    fn parses_terminal_choice_prompt() {
        let output = "Claude has written up a plan and is ready to execute. Would you like to\nproceed?\n❯ 1. Yes, clear context and bypass permissions\n2. Yes, and bypass permissions\n3. Yes, manually approve edits\n4. Type here to tell Claude what to change";
        let parsed = parse_terminal_choice_prompt(output).expect("prompt should parse");
        assert_eq!(parsed.questions.len(), 1);
        assert_eq!(parsed.questions[0].options.len(), 4);
        assert_eq!(
            parsed.questions[0].options[0].label,
            "Yes, clear context and bypass permissions"
        );
    }

    #[test]
    fn captures_plan_from_written_plan_file_tool_result() {
        let transcript = r##"{"type":"user","timestamp":"2026-02-22T18:00:02.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":"File created successfully at: /Users/me/.claude/plans/demo.md"}]},"toolUseResult":{"type":"create","filePath":"/Users/me/.claude/plans/demo.md","content":"# Plan\n\n## Step\n- Do one thing"},"uuid":"u1"}"##;
        let snapshot = parse_snapshot(&meta(), None, Some(transcript));
        let plan = snapshot.plan.expect("plan should exist");
        assert_eq!(plan.source, "toolUseResult");
        assert!(plan.markdown.unwrap_or_default().contains("# Plan"));
    }
}

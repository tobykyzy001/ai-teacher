use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::models::{LlmSettings, QType};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RETRIES: u32 = 3;

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".to_string(),
            content: content.to_string(),
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }
}

/// 业务用途与模型偏好的对应关系；连接地址和凭据仍统一使用主设置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmUse {
    Explain,
    KnowledgePoints,
    Quiz,
    Grading,
    Ask,
    Summarize,
    Ping,
}

/// 用途专用模型未设置、为 None 或仅为空白时回退主模型。
pub fn resolved_model<'a>(settings: &'a LlmSettings, usage: LlmUse) -> &'a str {
    let preferred = match usage {
        LlmUse::Explain | LlmUse::KnowledgePoints | LlmUse::Ask | LlmUse::Summarize => {
            settings.reading_model.as_deref()
        }
        LlmUse::Quiz => settings.quiz_model.as_deref(),
        LlmUse::Grading => settings.grading_model.as_deref(),
        LlmUse::Ping => None,
    };
    preferred
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(settings.model.as_str())
}

/// 环境变量 AI_TEACHER_MOCK_LLM=1 时跳过 HTTP，返回确定性假响应（调用时检查）。
pub fn is_mock_enabled() -> bool {
    std::env::var("AI_TEACHER_MOCK_LLM")
        .ok()
        .as_deref()
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// OpenAI 兼容协议的思考参数：关闭 = `reasoning_effort: none`，开启 = `medium`。
/// 厂商方言（enable_thinking / thinking 对象等）留待以后按需做独立协议适配，这里不掺。
pub fn thinking_params(disable_thinking: bool) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    map.insert(
        "reasoning_effort".to_string(),
        serde_json::Value::String(if disable_thinking { "none" } else { "medium" }.to_string()),
    );
    map
}

/// 部分网关把模型的思考过程直接拼进 `content`（标准 `reasoning_content`/`reasoning`
/// 字段我们根本不读），思考段以这些标记结尾；出现标记即说明之前全是思考。
const THINK_END_TAGS: [&str; 3] = ["</think>", "</thinking>", "</reasoning>"];

/// 文本中最早出现的思考结束标记的末尾字节位置。
fn thinking_end(text: &str) -> Option<usize> {
    THINK_END_TAGS
        .iter()
        .filter_map(|tag| text.find(tag).map(|start| (start, start + tag.len())))
        .min_by_key(|(start, _)| *start)
        .map(|(_, end)| end)
}

/// 去掉思考前缀：`We need answer...</think>2` → `2`；没有结束标记时原样返回。
pub(crate) fn strip_thinking(text: &str) -> String {
    match thinking_end(text) {
        Some(end) => text[end..].trim_start().to_string(),
        None => text.to_string(),
    }
}

/// 是否内联思考按 (base_url, model, 关思考开关) 记住结论：首次未知时先压住前导内容，
/// 等结束标记或流结束再决定，避免把思考当答案送出去；之后按结论直接流式放行。
fn inline_thinking_cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn inline_cache_key(base_url: &str, model: &str, disable_thinking: bool) -> String {
    format!("{base_url}|{model}|{disable_thinking}")
}

fn known_inline_thinking(key: &str) -> Option<bool> {
    inline_thinking_cache().lock().ok()?.get(key).copied()
}

fn remember_inline_thinking(key: &str, inline: bool) {
    if let Ok(mut cache) = inline_thinking_cache().lock() {
        cache.insert(key.to_string(), inline);
    }
}

/// 模型不接受思考参数时的一次性提示（内存去重，重启后可再提示）。
#[derive(Default)]
struct ThinkingNoticeState {
    seen: HashSet<String>,
    pending: Vec<String>,
}

impl ThinkingNoticeState {
    /// 同一个 (base_url, model, 开/关) 只记一次；返回是否新记录。
    fn record(&mut self, key: &str, message: String) -> bool {
        if !self.seen.insert(key.to_string()) {
            return false;
        }
        self.pending.push(message);
        true
    }

    fn take(&mut self) -> Option<String> {
        self.pending.pop()
    }
}

fn thinking_notice_state() -> &'static Mutex<ThinkingNoticeState> {
    static STATE: OnceLock<Mutex<ThinkingNoticeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(ThinkingNoticeState::default()))
}

/// 取走一条待展示的提示（供前端在 AI 动作结束后询问）。
pub fn take_thinking_notice() -> Option<String> {
    thinking_notice_state()
        .lock()
        .ok()
        .and_then(|mut s| s.take())
}

fn remember_thinking_rejected(base_url: &str, model: &str, disable_thinking: bool) {
    let key = format!("{base_url}|{model}|{disable_thinking}");
    let message = format!(
        "当前模型不支持{}思考，已按模型默认继续",
        if disable_thinking { "关闭" } else { "开启" }
    );
    if let Ok(mut state) = thinking_notice_state().lock() {
        state.record(&key, message);
    }
}

/// 流式输出的思考剥离器。已知不内联思考时原样放行；否则先压住内容，
/// 见到结束标记就把标记之前的部分丢掉、之后的内容照常流式发出。
struct AnswerFilter {
    hold: String,
    answer_started: bool,
    trim_pending: bool,
    inline_thinking: Option<bool>,
}

impl AnswerFilter {
    fn new(known_inline: Option<bool>) -> Self {
        Self {
            hold: String::new(),
            answer_started: known_inline == Some(false),
            trim_pending: false,
            inline_thinking: known_inline,
        }
    }

    fn push(&mut self, piece: &str) -> Vec<String> {
        if self.answer_started {
            return self.emit(piece);
        }
        self.hold.push_str(piece);
        let Some(end) = thinking_end(&self.hold) else {
            return Vec::new();
        };
        self.answer_started = true;
        self.inline_thinking = Some(true);
        self.trim_pending = true;
        let tail = self.hold[end..].to_string();
        self.hold.clear();
        self.emit(&tail)
    }

    /// 流结束时调用：没见到结束标记说明这段内容就是答案，整段放行。
    fn finish(&mut self) -> Vec<String> {
        if self.answer_started {
            return Vec::new();
        }
        self.inline_thinking = Some(false);
        let held = std::mem::take(&mut self.hold);
        if held.trim().is_empty() {
            Vec::new()
        } else {
            vec![held]
        }
    }

    fn emit(&mut self, piece: &str) -> Vec<String> {
        if piece.is_empty() {
            return Vec::new();
        }
        if self.trim_pending {
            let trimmed = piece.trim_start();
            if trimmed.is_empty() {
                return Vec::new();
            }
            self.trim_pending = false;
            return vec![trimmed.to_string()];
        }
        vec![piece.to_string()]
    }
}

/// OpenAI 兼容的 chat/completions 调用。base_url 是否以 /v1 结尾均可。
pub async fn chat(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    disable_thinking: bool,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("请先在设置中配置 API Key".to_string());
    }
    let url = normalize_base_url(base_url);
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let base_body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });
    let mut with_params = base_body.clone();
    if let Some(obj) = with_params.as_object_mut() {
        obj.extend(thinking_params(disable_thinking));
    }
    let mut use_params = true;
    let mut dropped_params = false;

    let mut retries = 0u32;
    loop {
        let body = if use_params { &with_params } else { &base_body };
        let response = client
            .post(&url)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await;
        let resp = match response {
            Ok(r) => r,
            Err(e) => return Err(format!("网络错误：{e}")),
        };
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if matches!(status.as_u16(), 400 | 422) && use_params {
            // 模型不认 reasoning_effort：去掉参数重试一次，思考开关降级为模型默认
            use_params = false;
            dropped_params = true;
            continue;
        }
        let retriable = status.as_u16() == 429 || status.is_server_error();
        if retriable && retries < MAX_RETRIES {
            retries += 1;
            tokio::time::sleep(Duration::from_secs(1 << (retries - 1))).await;
            continue;
        }
        if !status.is_success() {
            return Err(format!(
                "AI 请求失败（HTTP {}）：{}",
                status.as_u16(),
                truncate(&text, 300)
            ));
        }
        if dropped_params {
            remember_thinking_rejected(base_url, model, disable_thinking);
        }
        let parsed: ChatResponse =
            serde_json::from_str(&text).map_err(|e| format!("解析 AI 响应失败：{e}"))?;
        let raw = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .filter(|s| !s.trim().is_empty());
        let Some(raw) = raw else {
            return Err("AI 返回内容为空".to_string());
        };
        let content = strip_thinking(&raw);
        if content.trim().is_empty() {
            return Err("AI 返回内容为空".to_string());
        }
        return Ok(content);
    }
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

/// OpenAI 兼容的流式 chat/completions 调用：增量内容经 on_delta 推送，返回完整文本。
/// 仅在拿到初始响应前重试 429/5xx；流中途出错直接报错。
pub async fn chat_stream(
    settings: &LlmSettings,
    messages: Vec<ChatMessage>,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    chat_stream_for(settings, LlmUse::Ping, messages, on_delta).await
}

/// 流式请求内部入口：只按用途改变模型，不复制 endpoint、凭据或思考设置。
async fn chat_stream_for(
    settings: &LlmSettings,
    usage: LlmUse,
    messages: Vec<ChatMessage>,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let model = resolved_model(settings, usage);
    if settings.api_key.trim().is_empty() {
        return Err("请先在设置中配置 API Key".to_string());
    }
    let url = normalize_base_url(&settings.base_url);
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let base_body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
    });
    let mut with_params = base_body.clone();
    if let Some(obj) = with_params.as_object_mut() {
        obj.extend(thinking_params(settings.disable_thinking));
    }
    let mut use_params = true;
    let mut dropped_params = false;

    let mut retries = 0u32;
    let resp = loop {
        let body = if use_params { &with_params } else { &base_body };
        let response = client
            .post(&url)
            .bearer_auth(&settings.api_key)
            .json(body)
            .send()
            .await;
        let resp = match response {
            Ok(r) => r,
            Err(e) => return Err(format!("网络错误：{e}")),
        };
        let status = resp.status();
        if matches!(status.as_u16(), 400 | 422) && use_params {
            // 模型不认 reasoning_effort：去掉参数重试一次，思考开关降级为模型默认
            use_params = false;
            dropped_params = true;
            let _ = resp.text().await;
            continue;
        }
        let retriable = status.as_u16() == 429 || status.is_server_error();
        if retriable && retries < MAX_RETRIES {
            retries += 1;
            tokio::time::sleep(Duration::from_secs(1 << (retries - 1))).await;
            continue;
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "AI 请求失败（HTTP {}）：{}",
                status.as_u16(),
                truncate(&text, 300)
            ));
        }
        break resp;
    };
    if dropped_params {
        remember_thinking_rejected(&settings.base_url, model, settings.disable_thinking);
    }

    let cache_key = inline_cache_key(&settings.base_url, model, settings.disable_thinking);
    let mut filter = AnswerFilter::new(known_inline_thinking(&cache_key));
    let mut stream = resp.bytes_stream();
    // 只按完整行解码，原始字节先入缓冲，避免切在多字节字符中间
    let mut pending: Vec<u8> = Vec::new();
    let mut full = String::new();
    let mut done = false;
    while !done {
        match stream.next().await {
            Some(Ok(bytes)) => {
                pending.extend_from_slice(&bytes);
                while !done {
                    let Some(pos) = pending.iter().position(|&b| b == b'\n') else {
                        break;
                    };
                    let rest = pending.split_off(pos + 1);
                    let mut line_bytes = std::mem::replace(&mut pending, rest);
                    line_bytes.pop(); // 去掉行尾 \n
                    let line = String::from_utf8_lossy(&line_bytes);
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data:") {
                        match parse_sse_data(data) {
                            Some(SseEvent::Done) => done = true,
                            Some(SseEvent::Delta(piece)) => {
                                for out in filter.push(&piece) {
                                    on_delta(&out);
                                    full.push_str(&out);
                                }
                            }
                            None => {}
                        }
                    }
                }
            }
            Some(Err(e)) => return Err(format!("流式读取中断：{e}")),
            None => break,
        }
    }
    for out in filter.finish() {
        on_delta(&out);
        full.push_str(&out);
    }
    if let Some(inline) = filter.inline_thinking {
        remember_inline_thinking(&cache_key, inline);
    }
    if full.trim().is_empty() {
        return Err("AI 返回内容为空".to_string());
    }
    Ok(full)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SseEvent {
    Delta(String),
    Done,
}

/// 解析一行 SSE 的 data 负载："[DONE]" 结束帧、带内容的 delta 正常返回，其余（含垃圾行）返回 None。
fn parse_sse_data(data: &str) -> Option<SseEvent> {
    let payload = data.trim();
    if payload == "[DONE]" {
        return Some(SseEvent::Done);
    }
    let frame: SseFrame = serde_json::from_str(payload).ok()?;
    let content = frame
        .choices
        .into_iter()
        .next()?
        .delta
        .content
        .unwrap_or_default();
    if content.is_empty() {
        None
    } else {
        Some(SseEvent::Delta(content))
    }
}

#[derive(Debug, Deserialize)]
struct SseFrame {
    #[serde(default)]
    choices: Vec<SseChoice>,
}

#[derive(Debug, Deserialize)]
struct SseChoice {
    delta: SseDelta,
}

#[derive(Debug, Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.trim().to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
}

// ---------- 各业务用途 ----------

const MOCK_EXPLAIN: &str =
    "# 背景引入\n<mock 讲解>\n\n## 重点梳理\n- 要点一\n- 要点二\n\n## 难点解释\n<mock>";

/// mock 模式下把文本按字符边界切成 chunk_chars 一段，逐段推给回调，返回完整文本。
fn stream_mock_text(text: &str, chunk_chars: usize, on_delta: &mut dyn FnMut(&str)) -> String {
    let mut full = String::new();
    for chunk in text
        .chars()
        .collect::<Vec<char>>()
        .chunks(chunk_chars.max(1))
    {
        let piece: String = chunk.iter().collect();
        on_delta(&piece);
        full.push_str(&piece);
    }
    full
}

/// mock 模式下把文本切成约 n 段推送。
fn stream_mock_n_chunks(text: &str, n: usize, on_delta: &mut dyn FnMut(&str)) -> String {
    let chunk = text.chars().count().div_ceil(n).max(1);
    stream_mock_text(text, chunk, on_delta)
}

/// 前一单元上下文（可选）：拼进用户消息开头，让讲解衔接上一单元。
fn prev_context_section(prev_context: Option<&str>) -> String {
    prev_context
        .map(|c| format!("【前一单元内容概要】\n{c}\n\n"))
        .unwrap_or_default()
}

// 每次请求重新构造对应的中文段落，不复用上一轮的消息或可变缓冲区。
const READING_STYLE_PROMPTS: [(&str, &str); 4] = [
    (
        "default",
        "采用自然、耐心的领读风格，讲解清晰并帮助理解内容之间的联系。",
    ),
    (
        "exam",
        "采用应试导向的领读风格，聚焦考点、易错点和可检验的知识结论，必要时指出答题依据。",
    ),
    (
        "popular",
        "采用科普式的领读风格，用生活化类比和具体例子解释概念，减少不必要的术语。",
    ),
    (
        "socratic",
        "采用苏格拉底式领读，在讲解中穿插有针对性的引导问题，先启发学生思考，再给出明确答案。",
    ),
];
const READING_DEPTH_PROMPTS: [(&str, &str); 3] = [
    (
        "concise",
        "讲解简明扼要，只保留主线、核心概念和必要说明，避免铺陈。",
    ),
    (
        "standard",
        "讲解详略适中，说明核心概念、原因与主要内容之间的联系。",
    ),
    (
        "deep",
        "讲解深入充分，分析原理、推理过程、例子及必要的相关知识，同时保持条理清晰。",
    ),
];

fn reading_preference_prompt<'a>(prompts: &'a [(&str, &str)], preference: &str) -> &'a str {
    prompts
        .iter()
        .find_map(|(key, prompt)| (*key == preference.trim()).then_some(*prompt))
        .unwrap_or(prompts[0].1)
}

fn append_reading_preferences(prompt: &mut String, settings: &LlmSettings) {
    prompt.push_str("\n\n【本次领读风格】\n");
    prompt.push_str(reading_preference_prompt(
        &READING_STYLE_PROMPTS,
        &settings.reading_style,
    ));
    prompt.push_str("\n\n【本次讲解深度】\n");
    prompt.push_str(reading_preference_prompt(
        &READING_DEPTH_PROMPTS,
        &settings.reading_depth,
    ));
    prompt.push_str("\n这些领读偏好只影响本次表达，不改变知识事实或忽略指定的小节结构。");
}

fn explain_messages(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
) -> Vec<ChatMessage> {
    let mut prompt = format!(
        "{prev}请阅读下面的学习内容，写一篇 Markdown 格式的讲解，必须包含以下三个小节：\n\
         「## 背景引入」：用一两段话介绍这章内容在讲什么、为什么值得学；\n\
         「## 重点梳理」：用无序列表列出 3-6 个核心要点；\n\
         「## 难点解释」：挑出 1-3 个最难理解的概念，用通俗的方式解释。\n\n\
         学习内容：\n{unit_text}",
        prev = prev_context_section(prev_context)
    );
    append_reading_preferences(&mut prompt, settings);
    vec![
        ChatMessage::system("你是一位耐心、风趣的中文学习助教，擅长把材料讲得通俗易懂。"),
        ChatMessage::user(&prompt),
    ]
}

fn explain_chapter_messages(
    settings: &LlmSettings,
    chapter_title: &str,
    chapter_text: &str,
    prev_context: Option<&str>,
) -> Vec<ChatMessage> {
    let mut prompt = format!(
        "{prev}下面是一整章（《{title}》）的完整内容，请通读全章后写一篇 Markdown 格式的讲解，必须包含以下四个小节：\n\
         「## 背景引入」：用一两段话介绍这章在讲什么、为什么值得学；\n\
         「## 本章脉络」：梳理全章的内容结构，各部分如何衔接递进；\n\
         「## 重点梳理」：用无序列表列出 3-6 个核心要点；\n\
         「## 难点解释」：挑出 1-3 个最难理解的概念，用通俗的方式解释。\n\n\
         章节内容：\n{chapter_text}",
        prev = prev_context_section(prev_context),
        title = chapter_title
    );
    append_reading_preferences(&mut prompt, settings);
    vec![
        ChatMessage::system("你是一位耐心、风趣的中文学习助教，擅长把材料讲得通俗易懂。"),
        ChatMessage::user(&prompt),
    ]
}

fn ask_messages(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
    question: &str,
) -> Vec<ChatMessage> {
    let mut prompt = format!(
        "{prev}学习内容：\n{unit_text}\n\n学生的问题：{question}\n\n请基于学习内容回答问题，用中文。",
        prev = prev_context_section(prev_context)
    );
    append_reading_preferences(&mut prompt, settings);
    vec![
        ChatMessage::system("你是一位耐心的中文学习助教，回答要准确、简洁。"),
        ChatMessage::user(&prompt),
    ]
}

fn ask_selection_messages(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
    quote: &str,
    question: &str,
) -> Vec<ChatMessage> {
    let mut prompt = format!(
        "{prev}学习内容：\n{unit_text}\n\n【用户选中的原文】\n{quote}\n\n学生的问题：{question}\n\n请基于学习内容和选中的原文回答问题，用中文。",
        prev = prev_context_section(prev_context)
    );
    append_reading_preferences(&mut prompt, settings);
    vec![
        ChatMessage::system("你是一位耐心的中文学习助教，回答要准确、简洁。"),
        ChatMessage::user(&prompt),
    ]
}

pub async fn explain_stream(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(stream_mock_text(MOCK_EXPLAIN, 5, on_delta));
    }
    chat_stream_for(
        settings,
        LlmUse::Explain,
        explain_messages(settings, unit_text, prev_context),
        on_delta,
    )
    .await
}

const MOCK_CHAPTER_EXPLAIN: &str =
    "# 背景引入\n<mock 章讲解>\n\n## 本章脉络\n<mock>\n\n## 重点梳理\n- 要点一\n- 要点二\n\n## 难点解释\n<mock>";

/// 整章领读：chapter_text 为章内各小节拼接的正文（调用方已按需截断）。
pub async fn explain_chapter_stream(
    settings: &LlmSettings,
    chapter_title: &str,
    chapter_text: &str,
    prev_context: Option<&str>,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(stream_mock_text(MOCK_CHAPTER_EXPLAIN, 5, on_delta));
    }
    chat_stream_for(
        settings,
        LlmUse::Explain,
        explain_chapter_messages(settings, chapter_title, chapter_text, prev_context),
        on_delta,
    )
    .await
}

pub async fn knowledge_points(
    settings: &LlmSettings,
    unit_text: &str,
) -> Result<Vec<String>, String> {
    let raw = if is_mock_enabled() {
        "[\"知识点 A\",\"知识点 B\",\"知识点 C\"]".to_string()
    } else {
        chat(
            &settings.base_url,
            &settings.api_key,
            resolved_model(settings, LlmUse::KnowledgePoints),
            vec![
                ChatMessage::system("你是一位严谨的中文学习助教，只输出 JSON。"),
                ChatMessage::user(&format!(
                    "请从下面的学习内容中提炼 3 到 7 个最重要的知识点。每个知识点用一句简短的中文短语概括（10 到 20 字）。\
                     严格只输出一个 JSON 字符串数组，例如 [\"知识点一\",\"知识点二\"]，不要输出任何其他文字。\n\n\
                     学习内容：\n{unit_text}"
                )),
            ],
            settings.disable_thinking,
        )
        .await?
    };
    let points: Vec<String> = parse_lenient(&raw)?;
    if points.is_empty() {
        return Err("AI 未返回知识点，请重试".to_string());
    }
    Ok(points)
}

/// LLM 原始返回的题目（未校验形状）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawQuizQuestion {
    pub qtype: String,
    pub stem: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub knowledge_point: String,
}

/// 校验后的题目草稿，可直接入库。
#[derive(Debug, Clone)]
pub struct QuizDraft {
    pub qtype: QType,
    pub stem: String,
    pub options: Vec<String>,
    pub answer: String,
    pub explanation: String,
    pub knowledge_point: String,
}

/// 按单元正文字数规划题量：约每 500 字 1 题，夹在 3..=8 题；单选:简答 ≈ 3:2。
/// 返回 (单选题数, 简答题数)。
pub fn plan_quiz(chars: usize) -> (usize, usize) {
    let total = (chars / 500).clamp(3, 8);
    let mcq = (total * 3 + 2) / 5;
    (mcq, total - mcq)
}

/// 问答题考的是把知识用在一个具体对象上，而不是把知识点换一种句式复述。
/// 单独构造消息以便回归测试真实提示词，而不依赖线上模型碰巧生成好题。
fn quiz_messages(
    unit_text: &str,
    knowledge_points: &[String],
    mcq_n: usize,
    short_n: usize,
) -> Vec<ChatMessage> {
    let total = mcq_n + short_n;
    let kps = if knowledge_points.is_empty() {
        "（未提供）".to_string()
    } else {
        knowledge_points.join("、")
    };
    vec![
        ChatMessage::system("你是一位善于用问题启发思考的中文学习伙伴，只输出 JSON。"),
        ChatMessage::user(&format!(
            "请基于下面的学习内容和知识点，出一套共 {total} 道题的中文测验：{mcq_n} 道单选题和 {short_n} 道简答题（问答题）。\
             严格只输出一个 JSON 数组，共 {total} 个对象，每个对象的字段为：\n\
             qtype：\"mcq\"（单选）或 \"short\"（问答）；\n\
             stem：题干；\n\
             options：单选题为恰好 4 个选项（形如 \"A. xxx\"），问答题为空数组；\n\
             answer：单选题为正确选项字母（如 \"A\"），问答题为一个自然、完整的示范回答；\n\
             explanation：答案解析；\n\
             knowledge_point：题目考查的知识点（一个 10 到 20 字的短语，不要拼接多个要点）。\n\
             不要输出任何其他文字。\n\n\
             出题要求（务必遵守）：\n\
             1. 每道题只考查一个理解点，题干只能有一个明确的作答目标，不要把多个小问缝成一道。\n\
             2. 问答题要把知识放进一个与材料相符的具体情境或对象中，优先考查：解释一个现象为什么发生、\
             条件改变后会怎样、如何区分两个易混淆的概念，或根据给定证据作出判断。让学习者用日常语言说出\
             自己的理解即可，通常一两句话就能回答。若材料不适合虚构情境，就改用具体的反例或推导，\
             不要硬编与材料无关的专业背景。\n\
             3. 禁止把问答题写成背诵或写作任务：不要出\"请定义／简述／概括／总结／列举／列出…要点\"，\
             不要问\"有什么意义／作用／包含哪些方面\"，不要要求分点作答、使用固定术语或写一篇完整论述。\
             也不要出\"你觉得\"\"请谈谈你的看法\"这类没有明确判断依据的开放讨论题。\n\
             4. 问答题的 answer 用不超过 3 句自然的话给出关键结论和必要推理，可以包含合理的替代表述，\
             不能写成\"①…②…③…\"的背诵提纲。它只用于帮助理解，不要求学习者逐字或逐点覆盖。\n\
             5. 题目要能被单独判分：不依赖\"上面三种\"\"本节提到的\"这类上下文指代，题干自身说清对象、\
             条件和判断依据。涉及取舍时必须给出具体条件，不能靠应试套路猜出题人的心思。\n\n\
             问答题示例（仅为形式示例，不要沿用其中知识点）：小王以为调大缓存一定能减少重复计算，\
             但观察发现重复计算没有变化。结合缓存只保存相同输入结果的机制，他可能漏掉了什么条件？\n\
             示例答题：两次计算的输入可能并不相同；缓存只在输入相同时复用旧结果，单把缓存调大也无法命中。\n\n\
             知识点：{kps}\n\n\
             学习内容：\n{unit_text}"
        )),
    ]
}

pub async fn make_quiz(
    settings: &LlmSettings,
    unit_text: &str,
    knowledge_points: &[String],
) -> Result<Vec<RawQuizQuestion>, String> {
    let (mcq_n, short_n) = plan_quiz(unit_text.chars().count());
    let raw = if is_mock_enabled() {
        mock_quiz_json(knowledge_points, mcq_n, short_n)
    } else {
        chat(
            &settings.base_url,
            &settings.api_key,
            resolved_model(settings, LlmUse::Quiz),
            quiz_messages(unit_text, knowledge_points, mcq_n, short_n),
            settings.disable_thinking,
        )
        .await?
    };
    let quiz: Vec<RawQuizQuestion> = parse_lenient(&raw)?;
    if quiz.is_empty() {
        return Err("AI 未返回题目，请重试".to_string());
    }
    Ok(quiz)
}

/// 校验题目形状：mcq 必须恰好 4 个非空选项、short 选项为空、题干与答案非空。
/// 不满足返回 None（调用方可重试一次）。
pub fn normalize_quiz(raw: &[RawQuizQuestion]) -> Option<Vec<QuizDraft>> {
    if raw.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len());
    for q in raw {
        let qtype = QType::parse(&q.qtype).or_else(|| {
            if q.options.len() == 4 {
                Some(QType::Mcq)
            } else if q.options.is_empty() {
                Some(QType::Short)
            } else {
                None
            }
        })?;
        let stem = q.stem.trim().to_string();
        let mut answer = q.answer.trim().to_string();
        if stem.is_empty() || answer.is_empty() {
            return None;
        }
        let options = match qtype {
            QType::Mcq => {
                let opts: Vec<String> = q.options.iter().map(|o| o.trim().to_string()).collect();
                if opts.len() != 4 || opts.iter().any(|o| o.is_empty()) {
                    return None;
                }
                opts
            }
            QType::Short => {
                answer = answer.trim().to_string();
                Vec::new()
            }
        };
        let explanation = if q.explanation.trim().is_empty() {
            "（暂无解析）".to_string()
        } else {
            q.explanation.trim().to_string()
        };
        let knowledge_point = if q.knowledge_point.trim().is_empty() {
            "综合理解".to_string()
        } else {
            q.knowledge_point.trim().to_string()
        };
        out.push(QuizDraft {
            qtype,
            stem,
            options,
            answer,
            explanation,
            knowledge_point,
        });
    }
    Some(out)
}

fn mock_quiz_json(knowledge_points: &[String], mcq_n: usize, short_n: usize) -> String {
    let kp = |i: usize| -> String {
        knowledge_points
            .get(i)
            .cloned()
            .unwrap_or_else(|| match i % 3 {
                0 => "知识点 A".to_string(),
                1 => "知识点 B".to_string(),
                _ => "知识点 C".to_string(),
            })
    };
    let letters = ["A", "B", "C", "D"];
    let mut questions = Vec::new();
    for i in 0..mcq_n {
        let k = kp(i);
        let correct = letters[i % 4];
        questions.push(serde_json::json!({
            "qtype": "mcq",
            "stem": format!("关于「{k}」，下列说法正确的是哪一项？"),
            "options": letters.iter().map(|l| format!("{l}. {k} 的说法 {l}")).collect::<Vec<_>>(),
            "answer": correct,
            "explanation": format!("mock 解析：{correct} 是关于「{k}」最准确的表述。"),
            "knowledge_point": k,
        }));
    }
    for i in 0..short_n {
        let k = kp(i);
        questions.push(serde_json::json!({
            "qtype": "short",
            "stem": format!("同学说他已经了解「{k}」，却用它解释所有新现象，所得结论往往不成立。他的理解可能漏掉了哪项适用条件？"),
            "options": [],
            "answer": format!("「{k}」只在原文给出的条件下成立，不能脱离这些条件套用到所有现象；示范回答要结合原理解释这个边界。"),
            "explanation": "mock 解析：根据材料回到知识的适用条件，比复述定义更重要。",
            "knowledge_point": k,
        }));
    }
    serde_json::to_string_pretty(&questions).unwrap()
}

#[derive(Debug, Deserialize)]
struct GradeVerdict {
    correct: bool,
    #[serde(default)]
    comment: String,
}

fn grade_short_messages(
    unit_text: &str,
    stem: &str,
    reference: &str,
    given: &str,
) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("你是一位尊重学生思路的中文学习伙伴，只输出 JSON。"),
        ChatMessage::user(&format!(
            "请判断学生是否理解了这道问答题所考查的内容，并给他像同伴交流一样的具体反馈。\n\
             学习材料：\n{unit_text}\n\n\
             题目：{stem}\n\
             示范回答：{reference}\n\
             学生答案：{given}\n\
             判断标准：\n\
             1. 以题目条件和学习材料为依据，看学生是否真的说清了关键关系或推理，不是看他像不像示范回答。\n\
             2. 表达随意、很简短、用例子或类比都可以；口语、倒装、不完整句式不能成为扣分理由。\
             不要求固定术语、分点格式、篇幅或覆盖示范回答的每个措辞。直接照抄示范回答但没有理解内容时，不能算对。\n\
             3. 不同结论若有材料支持且言之有理，也可以判对；不能擅自增加题目没有的考试要求。\n\
             4. 结论和关键推理都合理才判 true。完全答非所问、关键关系错误或缺少必要依据才判 false；\
             只有次要遗漏或表达不完整时，不要为了凑齐要点判错。\n\
             5. comment 用 1–2 句自然中文，先具体指出学生理解得好的地方，再补充真正缺失或弄错的地方。\
             正确时不要复述示范回答、不要喊空泛口号；不正确时不要只说\"回答错误\"或\"请参考标准答案\"，\
             也不要堆砌考点编号或写一篇微型讲义。\n\
             严格只输出 JSON：{{\"correct\": true 或 false, \"comment\": \"给学生的具体反馈\"}}。"
        )),
    ]
}

pub async fn grade_short(
    settings: &LlmSettings,
    unit_text: &str,
    stem: &str,
    reference: &str,
    given: &str,
) -> Result<(bool, String), String> {
    let raw = if is_mock_enabled() {
        serde_json::json!({
            "correct": given.trim().len() >= 10,
            "comment": "mock 点评：说说你依据的关键条件和推理。",
        })
        .to_string()
    } else {
        chat(
            &settings.base_url,
            &settings.api_key,
            resolved_model(settings, LlmUse::Grading),
            grade_short_messages(unit_text, stem, reference, given),
            settings.disable_thinking,
        )
        .await?
    };
    let verdict: GradeVerdict = parse_lenient(&raw)?;
    let comment = if verdict.comment.trim().is_empty() {
        "（无点评）".to_string()
    } else {
        verdict.comment.trim().to_string()
    };
    Ok((verdict.correct, comment))
}

pub async fn ask_stream(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(stream_mock_n_chunks(
            &format!("mock 回答：{question}"),
            3,
            on_delta,
        ));
    }
    chat_stream_for(
        settings,
        LlmUse::Ask,
        ask_messages(settings, unit_text, prev_context, question),
        on_delta,
    )
    .await
}

pub async fn ask_selection_stream(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
    quote: &str,
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(stream_mock_n_chunks(
            &format!("mock 选段回答：{question}"),
            3,
            on_delta,
        ));
    }
    chat_stream_for(
        settings,
        LlmUse::Ask,
        ask_selection_messages(settings, unit_text, prev_context, quote, question),
        on_delta,
    )
    .await
}

pub async fn summarize(settings: &LlmSettings, unit_text: &str) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok("mock 摘要".to_string());
    }
    chat(
        &settings.base_url,
        &settings.api_key,
        resolved_model(settings, LlmUse::Summarize),
        vec![
            ChatMessage::system("你是一位严谨的中文学习助教，只输出摘要正文。"),
            ChatMessage::user(&format!(
                "请用 2~3 句话概括本单元核心内容（150字内），只输出摘要本身，不要任何其他文字。\n\n\
                 学习内容：\n{unit_text}"
            )),
        ],
        settings.disable_thinking,
    )
    .await
}

pub async fn ping(settings: &LlmSettings) -> Result<(), String> {
    if is_mock_enabled() {
        return Ok(());
    }
    chat(
        &settings.base_url,
        &settings.api_key,
        resolved_model(settings, LlmUse::Ping),
        vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("请只回复两个字：成功"),
        ],
        settings.disable_thinking,
    )
    .await
    .map(|_| ())
}

// ---------- 宽松 JSON 解析 ----------

fn strip_code_fences(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let after_lang = match rest.find('\n') {
            Some(nl) => &rest[nl + 1..],
            None => rest,
        };
        return after_lang.trim_end().trim_end_matches("```").trim();
    }
    t
}

fn extract_bracketed(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)?;
    let end = text.rfind(close)?;
    if end > start {
        Some(&text[start..=end])
    } else {
        None
    }
}

fn parse_lenient<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    let candidates = [raw.trim(), strip_code_fences(raw)];
    for cand in candidates {
        if let Ok(v) = serde_json::from_str::<T>(cand) {
            return Ok(v);
        }
    }
    let cleaned = strip_code_fences(raw);
    if let Some(arr) = extract_bracketed(cleaned, '[', ']') {
        if let Ok(v) = serde_json::from_str::<T>(arr) {
            return Ok(v);
        }
    }
    if let Some(obj) = extract_bracketed(cleaned, '{', '}') {
        if let Ok(v) = serde_json::from_str::<T>(obj) {
            return Ok(v);
        }
    }
    Err("AI 返回的内容不是合法的 JSON，请重试".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_settings() -> LlmSettings {
        LlmSettings {
            base_url: "http://127.0.0.1:9".to_string(),
            api_key: String::new(),
            model: "test-model".to_string(),
            auto_start_reading: false,
            disable_thinking: false,
            ..LlmSettings::default()
        }
    }

    fn reading_settings(style: &str, depth: &str) -> LlmSettings {
        LlmSettings {
            reading_style: style.to_string(),
            reading_depth: depth.to_string(),
            ..mock_settings()
        }
    }

    fn all_reading_message_builders(
    ) -> Vec<(&'static str, Box<dyn Fn(&LlmSettings) -> Vec<ChatMessage>>)> {
        vec![
            (
                "explain",
                Box::new(|s| explain_messages(s, "阅读内容", None)),
            ),
            (
                "explain_chapter",
                Box::new(|s| explain_chapter_messages(s, "章标题", "阅读内容", None)),
            ),
            (
                "ask",
                Box::new(|s| ask_messages(s, "阅读内容", None, "问题")),
            ),
            (
                "ask_selection",
                Box::new(|s| ask_selection_messages(s, "阅读内容", None, "选中句", "问题")),
            ),
        ]
    }

    /// 连续调用真实的 prompt 构造函数：上一轮的消息不参与下一轮构造。
    fn consecutive_seeded_prompts(
        build: impl Fn(&LlmSettings) -> Vec<ChatMessage>,
        first_seed: &str,
    ) -> (String, String) {
        let socratic = reading_settings("socratic", "deep");
        let mut first = build(&socratic);
        first[1].content.push_str(first_seed);
        let default = reading_settings("default", "standard");
        let second = build(&default);
        (first.remove(1).content, second[1].content.clone())
    }

    #[test]
    fn resolved_model_covers_every_business_use() {
        let settings = LlmSettings {
            reading_model: Some("阅读模型".to_string()),
            quiz_model: Some("出题模型".to_string()),
            grading_model: Some("判分模型".to_string()),
            ..mock_settings()
        };
        let expected = [
            (LlmUse::Explain, "阅读模型", "讲解请求"),
            (LlmUse::KnowledgePoints, "阅读模型", "知识点请求"),
            (LlmUse::Quiz, "出题模型", "出题请求"),
            (LlmUse::Grading, "判分模型", "判分请求"),
            (LlmUse::Ask, "阅读模型", "答疑请求"),
            (LlmUse::Summarize, "阅读模型", "摘要请求"),
            (LlmUse::Ping, "test-model", "连接测试"),
        ];
        for (usage, model, prompt) in expected {
            let message = ChatMessage::user(prompt);
            assert_eq!(message.role, "user");
            assert_eq!(message.content, prompt);
            assert_eq!(resolved_model(&settings, usage), model);
        }
    }

    #[test]
    fn absent_and_blank_model_preferences_fall_back_to_primary() {
        let mut settings = mock_settings();
        for usage in [
            LlmUse::Explain,
            LlmUse::KnowledgePoints,
            LlmUse::Quiz,
            LlmUse::Grading,
            LlmUse::Ask,
            LlmUse::Summarize,
            LlmUse::Ping,
        ] {
            assert_eq!(resolved_model(&settings, usage), "test-model");
        }
        settings.reading_model = Some("  阅读模型  ".to_string());
        assert_eq!(
            resolved_model(&settings, LlmUse::Explain),
            "阅读模型",
            "已配置的外围空白不应发给 API",
        );
        settings.reading_model = Some(String::new());
        settings.quiz_model = Some(" \n ".to_string());
        assert_eq!(resolved_model(&settings, LlmUse::Explain), "test-model");
        assert_eq!(resolved_model(&settings, LlmUse::Quiz), "test-model");
    }

    #[test]
    fn all_styles_and_depths_are_independent_utf8_prompt_segments() {
        let styles = [
            ("default", "自然、耐心"),
            ("exam", "应试导向"),
            ("popular", "科普式"),
            ("socratic", "苏格拉底式"),
        ];
        for (style, needle) in styles {
            let settings = reading_settings(style, "deep");
            let messages = explain_messages(&settings, "中文学习内容", None);
            assert_eq!(messages.len(), 2);
            let prompt = &messages[1].content;
            assert!(prompt.contains(needle), "缺少风格 {style}：{prompt}");
            let style_section = prompt
                .split_once("【本次领读风格】\n")
                .and_then(|(_, rest)| rest.split_once("\n\n【本次讲解深度】"))
                .map(|(section, _)| section)
                .expect("风格应是独立段落");
            assert!(style_section.contains(needle));
            assert!(!style_section.contains("深入充分"));
        }
        for (depth, needle) in [
            ("concise", "简明扼要"),
            ("standard", "详略适中"),
            ("deep", "深入充分"),
        ] {
            let settings = reading_settings("exam", depth);
            let prompt = explain_messages(&settings, "中文学习内容", None)[1]
                .content
                .clone();
            let depth_section = prompt.split_once("【本次讲解深度】\n").unwrap().1;
            assert!(depth_section.contains(needle), "深度 {depth}：{prompt}");
            assert!(depth_section.contains("这些领读偏好只影响本次表达"));
        }
        // 缺省值与未知/空白值均稳定回退，不残留前一次请求内容。
        assert_eq!(
            reading_preference_prompt(&READING_STYLE_PROMPTS, ""),
            READING_STYLE_PROMPTS[0].1
        );
        assert_eq!(
            reading_preference_prompt(&READING_DEPTH_PROMPTS, "unknown"),
            READING_DEPTH_PROMPTS[0].1
        );
    }

    #[test]
    fn all_reading_prompt_helpers_inject_preferences() {
        let settings = reading_settings("popular", "concise");
        for (_, build) in all_reading_message_builders() {
            let messages = build(&settings);
            assert_eq!(messages.len(), 2);
            let message = &messages[1];
            assert_eq!(message.role, "user");
            assert!(message.content.contains("采用科普式的领读风格"));
            assert!(message.content.contains("讲解简明扼要"));
        }
    }

    #[test]
    fn seeded_style_does_not_leak_into_the_next_prompt() {
        for (name, build) in all_reading_message_builders() {
            let (first, second) = consecutive_seeded_prompts(build, "上一轮独有标记");
            assert!(first.contains("苏格拉底式"), "{name}: {first}");
            assert!(first.contains("深入充分"));
            assert!(second.contains("自然、耐心"), "{name}: {second}");
            assert!(second.contains("详略适中"));
            assert!(!second.contains("苏格拉底式"), "{name}: {second}");
            assert!(!second.contains("深入充分"), "{name}: {second}");
            assert!(!second.contains("上一轮独有标记"), "{name}: {second}");
        }
    }

    #[test]
    fn inherited_mock_settings_use_default_reading_preferences() {
        let settings = mock_settings();
        assert_eq!(settings.reading_style, "default");
        assert_eq!(settings.reading_depth, "standard");
        assert_eq!(settings.reading_model, None);
        assert_eq!(settings.quiz_model, None);
        assert_eq!(settings.grading_model, None);
    }

    #[test]
    fn legacy_settings_json_defaults_to_primary_and_standard_reading() {
        let settings: LlmSettings = serde_json::from_str(
            r#"{
                "base_url":"https://api.example.com",
                "api_key":"key",
                "model":"主模型",
                "reading_model":null,
                "quiz_model":"",
                "grading_model":"   "
            }"#,
        )
        .unwrap();
        assert_eq!(settings.reading_style, "default");
        assert_eq!(settings.reading_depth, "standard");
        for usage in [
            LlmUse::Explain,
            LlmUse::KnowledgePoints,
            LlmUse::Quiz,
            LlmUse::Grading,
            LlmUse::Ask,
            LlmUse::Summarize,
            LlmUse::Ping,
        ] {
            assert_eq!(resolved_model(&settings, usage), "主模型");
        }
    }

    #[test]
    fn quiz_prompt_requires_scenarios_not_recitation() {
        let prompt =
            &quiz_messages("缓存只在输入相同时命中。", &["缓存命中条件".into()], 3, 2)[1].content;
        for expected in [
            "具体情境或对象",
            "通常一两句话就能回答",
            "不要出\"请定义／简述／概括／总结／列举",
            "不要要求分点作答",
            "不要求学习者逐字或逐点覆盖",
        ] {
            assert!(
                prompt.contains(expected),
                "问答题约束缺失：{expected}\n{prompt}"
            );
        }
        for banned_old_wording in ["要点式参考答案", "有唯一判断标准的讨论题"] {
            assert!(
                !prompt.contains(banned_old_wording),
                "不应保留把问答题拉回标准答案的旧约束：{banned_old_wording}"
            );
        }
    }

    #[test]
    fn grading_prompt_assesses_understanding_with_source_context() {
        let messages = grade_short_messages(
            "输入相同才能复用缓存结果。",
            "为什么调大缓存仍未减少计算？",
            "输入可能不同。",
            "可能是输入变了。",
        );
        let prompt = &messages[1].content;
        assert!(prompt.contains("输入相同才能复用缓存结果。"));
        assert!(prompt.contains("可能是输入变了。"));
        for expected in [
            "不是看他像不像示范回答",
            "表达随意、很简短、用例子或类比都可以",
            "不同结论若有材料支持且言之有理，也可以判对",
            "只有次要遗漏或表达不完整时，不要为了凑齐要点判错",
        ] {
            assert!(
                prompt.contains(expected),
                "判分规则缺失：{expected}\n{prompt}"
            );
        }
        assert!(
            !prompt.contains("不要求措辞一致"),
            "不能只放松措辞而仍逐点照标准答案判分"
        );
    }

    #[tokio::test]
    async fn mock_explanation_streams_sections() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let mut deltas: Vec<String> = Vec::new();
        let out = explain_stream(&mock_settings(), "任意内容", None, &mut |d: &str| {
            deltas.push(d.to_string());
        })
        .await
        .unwrap();
        assert!(out.contains("背景引入"));
        assert!(out.contains("重点梳理"));
        assert!(out.contains("难点解释"));
        assert!(deltas.len() >= 2, "mock 讲解应分多段推送");
        assert_eq!(deltas.concat(), out);
    }

    #[tokio::test]
    async fn mock_explanation_with_prev_context() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let out = explain_stream(
            &mock_settings(),
            "任意内容",
            Some("《上一章》：上一章讲了基础概念"),
            &mut |_: &str| {},
        )
        .await
        .unwrap();
        assert!(out.contains("背景引入"));
    }

    #[tokio::test]
    async fn mock_chapter_explanation_streams_sections() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let mut deltas: Vec<String> = Vec::new();
        let out = explain_chapter_stream(
            &mock_settings(),
            "测试章",
            "整章内容",
            Some("《上一章》：概要"),
            &mut |d: &str| deltas.push(d.to_string()),
        )
        .await
        .unwrap();
        assert!(out.contains("背景引入"));
        assert!(out.contains("本章脉络"));
        assert!(out.contains("重点梳理"));
        assert!(out.contains("难点解释"));
        assert!(deltas.len() >= 2, "mock 章讲解应分多段推送");
        assert_eq!(deltas.concat(), out);
    }

    #[tokio::test]
    async fn mock_knowledge_points() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let kps = knowledge_points(&mock_settings(), "任意内容")
            .await
            .unwrap();
        assert_eq!(kps, vec!["知识点 A", "知识点 B", "知识点 C"]);
    }

    #[tokio::test]
    async fn mock_quiz_shape() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let kps = vec!["阿尔法".to_string(), "贝塔".to_string(), "伽马".to_string()];
        // 2600 字 → 5 题（3 单选 + 2 简答），与旧固定题量一致
        let raw = make_quiz(&mock_settings(), &"内".repeat(2600), &kps)
            .await
            .unwrap();
        assert_eq!(raw.len(), 5);
        assert_eq!(raw.iter().filter(|q| q.qtype == "mcq").count(), 3);
        assert_eq!(raw.iter().filter(|q| q.qtype == "short").count(), 2);
        assert!(raw[..3].iter().all(|q| q.options.len() == 4));
        assert_eq!(raw[0].answer, "A");
        assert_eq!(raw[1].answer, "B");
        assert_eq!(raw[2].answer, "C");

        let drafts = normalize_quiz(&raw).unwrap();
        assert_eq!(drafts.len(), 5);
        assert_eq!(drafts[0].qtype, QType::Mcq);
        assert!(drafts.iter().any(|d| d.knowledge_point == "阿尔法"));
        assert!(drafts.iter().any(|d| d.knowledge_point == "伽马"));
        let shorts: Vec<_> = drafts.iter().filter(|d| d.qtype == QType::Short).collect();
        assert!(shorts.iter().all(|d| d.stem.contains("适用条件")));
        assert!(shorts.iter().all(|d| !d.stem.contains("简要说明")));
    }

    #[tokio::test]
    async fn mock_quiz_without_knowledge_points() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let raw = make_quiz(&mock_settings(), &"内".repeat(2600), &[])
            .await
            .unwrap();
        assert_eq!(raw.len(), 5);
        assert!(normalize_quiz(&raw).is_some());
    }

    #[test]
    fn quiz_plan_scales_with_length() {
        assert_eq!(plan_quiz(0), (2, 1)); // 下限 3 题
        assert_eq!(plan_quiz(1499), (2, 1));
        assert_eq!(plan_quiz(2000), (2, 2)); // 4 题
        assert_eq!(plan_quiz(2550), (3, 2)); // 5 题，与旧固定值一致
        assert_eq!(plan_quiz(3000), (4, 2)); // 6 题
        assert_eq!(plan_quiz(5100), (5, 3)); // 上限 8 题
    }

    #[tokio::test]
    async fn mock_grade_short() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let s = mock_settings();
        let (ok, comment) = grade_short(&s, "原文", "题干", "参考答案", "太短")
            .await
            .unwrap();
        assert!(!ok);
        assert_eq!(comment, "mock 点评：说说你依据的关键条件和推理。");
        let (ok, _) = grade_short(
            &s,
            "原文",
            "题干",
            "参考答案",
            "这是一段足够长的学生回答，阐述了核心含义",
        )
        .await
        .unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn mock_ask_streams_three_chunks() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let mut deltas: Vec<String> = Vec::new();
        let out = ask_stream(
            &mock_settings(),
            "内容",
            None,
            "什么是记忆？",
            &mut |d: &str| deltas.push(d.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(out, "mock 回答：什么是记忆？");
        assert_eq!(deltas.len(), 3);
        assert_eq!(deltas.concat(), out);
    }

    #[tokio::test]
    async fn mock_ask_with_prev_context() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let out = ask_stream(
            &mock_settings(),
            "内容",
            Some("《上一章》：概要"),
            "什么是记忆？",
            &mut |_: &str| {},
        )
        .await
        .unwrap();
        assert_eq!(out, "mock 回答：什么是记忆？");
    }

    #[tokio::test]
    async fn mock_ask_selection_streams() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let mut deltas: Vec<String> = Vec::new();
        let out = ask_selection_stream(
            &mock_settings(),
            "内容",
            Some("《上一章》：概要"),
            "选中的原文句子",
            "这句话什么意思？",
            &mut |d: &str| deltas.push(d.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(out, "mock 选段回答：这句话什么意思？");
        assert!(deltas.len() >= 2);
        assert_eq!(deltas.concat(), out);
    }

    #[tokio::test]
    async fn mock_summarize() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        assert_eq!(
            summarize(&mock_settings(), "任意内容").await.unwrap(),
            "mock 摘要"
        );
    }

    #[tokio::test]
    async fn mock_ping_ok() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        ping(&mock_settings()).await.unwrap();
    }

    /// 读一次完整的 HTTP 请求，返回请求体（JSON）。
    fn read_http_request(socket: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut req: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        let header_end = loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0, "客户端连接被提前关闭");
            req.extend_from_slice(&buf[..n]);
            if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&req[..header_end]).to_string();
        let content_length: usize = headers
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while req.len() < header_end + content_length {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            req.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&req[header_end..]).to_string()
    }

    fn write_http(socket: &mut std::net::TcpStream, status: &str, content_type: &str, body: &str) {
        use std::io::Write;
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(resp.as_bytes()).unwrap();
        socket.flush().unwrap();
    }

    #[test]
    fn thinking_params_follow_openai_protocol() {
        // 只按 OpenAI 兼容协议发 reasoning_effort：关闭 = none，开启 = medium
        let off = thinking_params(true);
        assert_eq!(
            off.get("reasoning_effort"),
            Some(&serde_json::Value::String("none".to_string()))
        );
        assert_eq!(off.len(), 1, "不应掺厂商方言参数：{off:?}");

        let on = thinking_params(false);
        assert_eq!(
            on.get("reasoning_effort"),
            Some(&serde_json::Value::String("medium".to_string()))
        );
        assert_eq!(on.len(), 1, "不应掺厂商方言参数：{on:?}");
    }

    #[test]
    fn thinking_notice_records_once_per_key() {
        let mut state = ThinkingNoticeState::default();
        assert!(state.record("api.example.com|m|off", "不支持关闭思考".to_string()));
        assert!(!state.record("api.example.com|m|off", "再次失败".to_string()));
        assert_eq!(state.take(), Some("不支持关闭思考".to_string()));
        assert_eq!(state.take(), None);
    }

    #[tokio::test]
    async fn chat_retries_without_thinking_params_on_400() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let body = read_http_request(&mut first);
            assert!(
                body.contains("\"reasoning_effort\":\"none\""),
                "首次请求应带关思考参数：{body}"
            );
            write_http(
                &mut first,
                "400 Bad Request",
                "application/json",
                r#"{"error":{"message":"Unsupported parameter: reasoning_effort"}}"#,
            );

            let (mut second, _) = listener.accept().unwrap();
            let body = read_http_request(&mut second);
            assert!(
                !body.contains("reasoning_effort"),
                "重试应去掉思考参数：{body}"
            );
            write_http(
                &mut second,
                "200 OK",
                "application/json",
                r#"{"choices":[{"message":{"content":"你好"}}]}"#,
            );
        });

        let text = chat(
            &format!("http://{addr}"),
            "sk-test",
            "retry-model",
            vec![ChatMessage::user("hi")],
            true,
        )
        .await
        .unwrap();
        assert_eq!(text, "你好");
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn chat_stream_retries_without_thinking_params_on_400() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let body = read_http_request(&mut first);
            assert!(
                body.contains("\"reasoning_effort\":\"none\""),
                "首次请求应带关思考参数：{body}"
            );
            write_http(
                &mut first,
                "400 Bad Request",
                "application/json",
                r#"{"error":{"message":"Unsupported parameter: reasoning_effort"}}"#,
            );

            let (mut second, _) = listener.accept().unwrap();
            let body = read_http_request(&mut second);
            assert!(
                !body.contains("reasoning_effort"),
                "重试应去掉思考参数：{body}"
            );
            let events = "data: {\"choices\":[{\"delta\":{\"content\":\"答\"}}]}\n\n\
                          data: {\"choices\":[{\"delta\":{\"content\":\"案\"}}]}\n\n\
                          data: [DONE]\n\n";
            write_http(&mut second, "200 OK", "text/event-stream", events);
        });

        let settings = LlmSettings {
            base_url: format!("http://{addr}"),
            api_key: "sk-test".to_string(),
            model: "stream-retry-model".to_string(),
            disable_thinking: true,
            ..LlmSettings::default()
        };
        let full = chat_stream(&settings, vec![ChatMessage::user("hi")], &mut |_d| {})
            .await
            .unwrap();
        assert_eq!(full, "答案");
        handle.join().unwrap();
    }

    /// 实际模型选择由求助处的 LlmUse 唯一决定，端点和 key 仍从主 settings 借用。
    /// 这里用测试消息验证无全局状态、也不会修改任何业务输入。
    #[test]
    fn preference_changes_do_not_mutate_settings_or_request_messages() {
        let settings = LlmSettings {
            reading_model: Some("阅读模型".to_string()),
            quiz_model: Some("出题模型".to_string()),
            grading_model: Some("判分模型".to_string()),
            reading_style: "exam".to_string(),
            reading_depth: "concise".to_string(),
            ..mock_settings()
        };
        let before = ChatMessage::user("中文测试问题");
        let before_json = serde_json::to_string(&before).unwrap();
        assert_eq!(resolved_model(&settings, LlmUse::Quiz), "出题模型");
        assert_eq!(resolved_model(&settings, LlmUse::Grading), "判分模型");
        assert_eq!(resolved_model(&settings, LlmUse::Explain), "阅读模型");
        let after = ChatMessage::user("中文测试问题");
        assert_eq!(serde_json::to_string(&after).unwrap(), before_json);
        assert_eq!(settings.model, "test-model");
        assert_eq!(settings.base_url, "http://127.0.0.1:9");
        assert_eq!(settings.api_key, "");
    }

    #[test]
    fn strip_thinking_drops_inline_reasoning_prefix() {
        // 部分网关把思考拼进 content，只给结束标记、不给开标签
        assert_eq!(strip_thinking("We need answer. 1+1=2.</think>2"), "2");
        assert_eq!(
            strip_thinking("思考…\n</think>\n\n## 重点梳理\n- 一"),
            "## 重点梳理\n- 一"
        );
        assert_eq!(strip_thinking("<thinking>想一想</thinking>答案"), "答案");
        assert_eq!(strip_thinking("<reasoning>想一想</reasoning>答案"), "答案");
        // 正常回答不含标记：原样保留，不能误删正文（含英文 response 一词也不能误伤）
        assert_eq!(strip_thinking("## 背景引入\n正文"), "## 背景引入\n正文");
        assert_eq!(
            strip_thinking("HTTP response 是服务端的应答"),
            "HTTP response 是服务端的应答"
        );
    }

    #[test]
    fn answer_filter_holds_reasoning_and_streams_answer() {
        // 未知模型：思考段压住不发，见到结束标记后的内容立即流式发出
        let mut filter = AnswerFilter::new(None);
        assert!(filter.push("We need ").is_empty());
        assert!(filter.push("answer. 1+1=").is_empty());
        assert_eq!(filter.push("2.</think>2"), vec!["2".to_string()]);
        assert_eq!(filter.push(" 个"), vec![" 个".to_string()]);
        assert!(filter.finish().is_empty());
        assert_eq!(filter.inline_thinking, Some(true));
    }

    #[test]
    fn answer_filter_flushes_plain_answer_at_end_of_stream() {
        // 未知模型但整段没有思考标记：流结束时整段放行，并记住这个模型不内联思考
        let mut filter = AnswerFilter::new(None);
        assert!(filter.push("## 背景引入").is_empty());
        assert!(filter.push("\n正文").is_empty());
        assert_eq!(filter.finish(), vec!["## 背景引入\n正文".to_string()]);
        assert_eq!(filter.inline_thinking, Some(false));
    }

    #[test]
    fn answer_filter_streams_directly_when_model_known_plain() {
        let mut filter = AnswerFilter::new(Some(false));
        assert_eq!(filter.push("第一段"), vec!["第一段".to_string()]);
        assert_eq!(filter.push("第二段"), vec!["第二段".to_string()]);
        assert!(filter.finish().is_empty());
    }

    #[test]
    fn parse_sse_data_reads_content_only() {
        // reasoning_content 是标准思考字段，绝不能当答案
        let frame = r#"{"choices":[{"delta":{"reasoning_content":"思考","content":null}}]}"#;
        assert_eq!(parse_sse_data(frame), None);
        let frame = r#"{"choices":[{"delta":{"content":"答案"}}]}"#;
        assert_eq!(
            parse_sse_data(frame),
            Some(SseEvent::Delta("答案".to_string()))
        );
    }

    #[tokio::test]
    async fn empty_api_key_rejected_before_any_request() {
        // chat() 不经过 mock 分支，空 key 直接报错，且不会发起网络请求
        let err = chat("", "", "m", vec![ChatMessage::user("hi")], false)
            .await
            .unwrap_err();
        assert_eq!(err, "请先在设置中配置 API Key");
    }

    #[tokio::test]
    async fn chat_stream_empty_api_key_rejected_before_any_request() {
        let err = chat_stream(
            &mock_settings(),
            vec![ChatMessage::user("hi")],
            &mut |_: &str| {},
        )
        .await
        .unwrap_err();
        assert_eq!(err, "请先在设置中配置 API Key");
    }

    /// 起一个只回一次 SSE 响应的本地 HTTP 服务，端到端验证流式解析
    /// （响应体分两段写出，段边界大概率切在多字节字符中间）。
    #[tokio::test]
    async fn chat_stream_parses_sse_from_local_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut req: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            let header_end = loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0, "客户端连接被提前关闭");
                req.extend_from_slice(&buf[..n]);
                if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&req[..header_end]).to_string();
            let content_length: usize = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while req.len() < header_end + content_length {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                req.extend_from_slice(&buf[..n]);
            }
            let body = String::from_utf8_lossy(&req[header_end..]).to_string();
            assert!(body.contains("\"stream\":true"), "请求体应为流式：{body}");

            let events = "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n\
                          data: {\"choices\":[{\"delta\":{\"content\":\"好，世界\"}}]}\n\n\
                          data: {\"choices\":[{\"delta\":{}}]}\n\n\
                          : keepalive comment\n\n\
                          data: [DONE]\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{events}",
                events.len()
            );
            let mid = response.len() / 2;
            socket.write_all(response[..mid].as_bytes()).unwrap();
            socket.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            socket.write_all(response[mid..].as_bytes()).unwrap();
            socket.flush().unwrap();
        });

        let settings = LlmSettings {
            base_url: format!("http://{addr}"),
            api_key: "sk-test".to_string(),
            model: "m".to_string(),
            disable_thinking: false,
            ..LlmSettings::default()
        };
        let mut deltas: Vec<String> = Vec::new();
        let full = chat_stream(&settings, vec![ChatMessage::user("hi")], &mut |d: &str| {
            deltas.push(d.to_string());
        })
        .await
        .unwrap();
        assert_eq!(full, "你好，世界");
        // 该模型首次出现（缓存未知）先压住内容、流结束再整段放行：
        // 分片边界不再固定，但拼接结果必须等于全文，且空 delta / 注释行不推送
        assert_eq!(deltas.concat(), full);
        assert!(
            deltas.iter().all(|d| !d.is_empty()),
            "空 delta 与注释行不应推送"
        );
        handle.join().unwrap();
    }

    #[test]
    fn sse_frame_parsing() {
        use SseEvent::{Delta, Done};
        assert_eq!(parse_sse_data("[DONE]"), Some(Done));
        assert_eq!(parse_sse_data("  [DONE]  "), Some(Done));
        assert_eq!(
            parse_sse_data(r#"{"choices":[{"delta":{"content":"你好"}}]}"#),
            Some(Delta("你好".to_string()))
        );
        assert_eq!(
            parse_sse_data(r#"{"choices":[{"delta":{"content":"a"}},{"delta":{"content":"b"}}]}"#),
            Some(Delta("a".to_string()))
        );
        // 空 content 或缺失 content 的 delta：没有可推送内容
        assert_eq!(
            parse_sse_data(r#"{"choices":[{"delta":{"content":""}}]}"#),
            None
        );
        assert_eq!(parse_sse_data(r#"{"choices":[{"delta":{}}]}"#), None);
        assert_eq!(parse_sse_data(r#"{"choices":[]}"#), None);
        // 垃圾行一律忽略
        assert_eq!(parse_sse_data("event: ping"), None);
        assert_eq!(parse_sse_data("不是 JSON"), None);
        assert_eq!(parse_sse_data(""), None);
    }

    #[test]
    fn mock_chunking_is_char_safe() {
        let mut deltas: Vec<String> = Vec::new();
        let full = stream_mock_text("你好世界ABC", 2, &mut |d| deltas.push(d.to_string()));
        assert_eq!(full, "你好世界ABC");
        assert_eq!(
            deltas,
            vec![
                "你好".to_string(),
                "世界".to_string(),
                "AB".to_string(),
                "C".to_string()
            ]
        );

        let mut three: Vec<String> = Vec::new();
        let full = stream_mock_n_chunks("一二三四五", 3, &mut |d| three.push(d.to_string()));
        assert_eq!(full, "一二三四五");
        assert_eq!(three.len(), 3);
        assert_eq!(three.concat(), full);
    }

    #[test]
    fn normalize_quiz_rejects_bad_shapes() {
        let good = |options: Vec<&str>| RawQuizQuestion {
            qtype: "mcq".to_string(),
            stem: "题干".to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            answer: "A".to_string(),
            explanation: "解析".to_string(),
            knowledge_point: "要点".to_string(),
        };
        assert!(normalize_quiz(&[good(vec!["A. 甲", "B. 乙", "C. 丙", "D. 丁"])]).is_some());
        assert!(normalize_quiz(&[good(vec!["A. 甲", "B. 乙", "C. 丙"])]).is_none());
        assert!(normalize_quiz(&[good(vec!["A. 甲", "B. 乙", "C. 丙", ""])]).is_none());
        let empty_stem = RawQuizQuestion {
            stem: "  ".to_string(),
            ..good(vec!["A", "B", "C", "D"])
        };
        assert!(normalize_quiz(&[empty_stem]).is_none());
        assert!(normalize_quiz(&[]).is_none());

        let short_like_mcq = RawQuizQuestion {
            qtype: "unknown".to_string(),
            options: vec![],
            stem: "题干".to_string(),
            answer: "参考".to_string(),
            explanation: "解析".to_string(),
            knowledge_point: "要点".to_string(),
        };
        let drafts = normalize_quiz(&[short_like_mcq]).unwrap();
        assert_eq!(drafts[0].qtype, QType::Short);
    }

    #[test]
    fn lenient_json_parsing() {
        let fenced = "好的，以下是题目：\n```json\n[{\"qtype\":\"mcq\",\"stem\":\"s\",\"options\":[\"A\",\"B\",\"C\",\"D\"],\"answer\":\"A\",\"explanation\":\"e\",\"knowledge_point\":\"k\"}]\n```\n祝学习愉快";
        let v: Vec<RawQuizQuestion> = parse_lenient(fenced).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].answer, "A");

        let direct: Vec<String> = parse_lenient("[\"a\",\"b\"]").unwrap();
        assert_eq!(direct, vec!["a", "b"]);

        let verdict: GradeVerdict =
            parse_lenient("结果：{\"correct\": true, \"comment\": \"不错\"}").unwrap();
        assert!(verdict.correct);

        assert!(parse_lenient::<Vec<String>>("完全没有 JSON").is_err());
    }

    #[test]
    fn base_url_normalization() {
        assert_eq!(
            normalize_base_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_base_url("https://api.deepseek.com/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_base_url("https://x.com/v1"),
            "https://x.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_base_url("https://x.com/v1/"),
            "https://x.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_base_url(" https://x.com/openai/v1 "),
            "https://x.com/openai/v1/chat/completions"
        );
    }
}

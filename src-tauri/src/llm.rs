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
    thinking_notice_state().lock().ok().and_then(|mut s| s.take())
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
    if settings.api_key.trim().is_empty() {
        return Err("请先在设置中配置 API Key".to_string());
    }
    let url = normalize_base_url(&settings.base_url);
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let base_body = serde_json::json!({
        "model": settings.model,
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
        remember_thinking_rejected(
            &settings.base_url,
            &settings.model,
            settings.disable_thinking,
        );
    }

    let cache_key = inline_cache_key(
        &settings.base_url,
        &settings.model,
        settings.disable_thinking,
    );
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

pub async fn explain_stream(
    settings: &LlmSettings,
    unit_text: &str,
    prev_context: Option<&str>,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(stream_mock_text(MOCK_EXPLAIN, 5, on_delta));
    }
    chat_stream(
        settings,
        vec![
            ChatMessage::system("你是一位耐心、风趣的中文学习助教，擅长把材料讲得通俗易懂。"),
            ChatMessage::user(&format!(
                "{prev}请阅读下面的学习内容，写一篇 Markdown 格式的讲解，必须包含以下三个小节：\n\
                 「## 背景引入」：用一两段话介绍这章内容在讲什么、为什么值得学；\n\
                 「## 重点梳理」：用无序列表列出 3-6 个核心要点；\n\
                 「## 难点解释」：挑出 1-3 个最难理解的概念，用通俗的方式解释。\n\n\
                 学习内容：\n{unit_text}",
                prev = prev_context_section(prev_context)
            )),
        ],
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
    chat_stream(
        settings,
        vec![
            ChatMessage::system("你是一位耐心、风趣的中文学习助教，擅长把材料讲得通俗易懂。"),
            ChatMessage::user(&format!(
                "{prev}下面是一整章（《{title}》）的完整内容，请通读全章后写一篇 Markdown 格式的讲解，必须包含以下四个小节：\n\
                 「## 背景引入」：用一两段话介绍这章在讲什么、为什么值得学；\n\
                 「## 本章脉络」：梳理全章的内容结构，各部分如何衔接递进；\n\
                 「## 重点梳理」：用无序列表列出 3-6 个核心要点；\n\
                 「## 难点解释」：挑出 1-3 个最难理解的概念，用通俗的方式解释。\n\n\
                 章节内容：\n{chapter_text}",
                prev = prev_context_section(prev_context),
                title = chapter_title
            )),
        ],
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
            &settings.model,
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

pub async fn make_quiz(
    settings: &LlmSettings,
    unit_text: &str,
    knowledge_points: &[String],
) -> Result<Vec<RawQuizQuestion>, String> {
    let (mcq_n, short_n) = plan_quiz(unit_text.chars().count());
    let total = mcq_n + short_n;
    let raw = if is_mock_enabled() {
        mock_quiz_json(knowledge_points, mcq_n, short_n)
    } else {
        let kps = if knowledge_points.is_empty() {
            "（未提供）".to_string()
        } else {
            knowledge_points.join("、")
        };
        chat(
            &settings.base_url,
            &settings.api_key,
            &settings.model,
            vec![
                ChatMessage::system("你是一位严谨的中文出题老师，只输出 JSON。"),
                ChatMessage::user(&format!(
                    "请基于下面的学习内容和知识点，出一套共 {total} 道题的中文测验：{mcq_n} 道单选题和 {short_n} 道简答题。\
                     严格只输出一个 JSON 数组，共 {total} 个对象，每个对象的字段为：\n\
                     qtype：\"mcq\"（单选）或 \"short\"（简答）；\n\
                     stem：题干；\n\
                     options：单选题为恰好 4 个选项（形如 \"A. xxx\"），简答题为空数组；\n\
                     answer：单选题为正确选项字母（如 \"A\"），简答题为参考答案；\n\
                     explanation：答案解析；\n\
                     knowledge_point：题目考查的知识点（一个 10 到 20 字的短语，不要拼接多个要点）。\n\
                     不要输出任何其他文字。\n\n\
                     出题要求（务必遵守）：\n\
                     1. 每道题只考查一个判断点。若某个知识点包含多个小问或需要多步取舍，拆成多道题，\
                     绝不要把几个小问合并成一道——题干里只能有问号或只能有一个明确的作答目标。\n\
                     2. 简答题的 answer 写成不超过 3 句的要点式参考答案，用陈述句，不要写成大段论述。\n\
                     3. 不要出开放式、无唯一判断标准的讨论题：禁止\"你觉得\"\"你会怎么选\"\"请谈谈你的看法\"\
                     \"请说明你的理由\"这类设问；需要考取舍时，把条件写进题干（给出具体阈值、负载或成本），\
                     让答案有明确的对错。\n\
                     4. 题目要能被单独判分：不依赖\"上面三种\"\"本节提到的\"这类上下文指代，题干自身说清对象。\n\n\
                     知识点：{kps}\n\n\
                     学习内容：\n{unit_text}"
                )),
            ],
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
            "stem": format!("请用自己的话简要说明「{k}」。"),
            "options": [],
            "answer": format!("{k} 的核心含义是……（参考答案）"),
            "explanation": "mock 解析：能说出核心含义即可。",
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

pub async fn grade_short(
    settings: &LlmSettings,
    stem: &str,
    reference: &str,
    given: &str,
) -> Result<(bool, String), String> {
    let raw = if is_mock_enabled() {
        serde_json::json!({
            "correct": given.trim().len() >= 10,
            "comment": "mock 点评",
        })
        .to_string()
    } else {
        chat(
            &settings.base_url,
            &settings.api_key,
            &settings.model,
            vec![
                ChatMessage::system("你是一位宽容但严格的中文阅卷老师，只输出 JSON。"),
                ChatMessage::user(&format!(
                    "请判断学生的简答题是否答对。\n\
                     题目：{stem}\n\
                     参考答案：{reference}\n\
                     学生答案：{given}\n\
                     判断标准：学生答案表达了参考答案的核心含义即算正确，不要求措辞一致。\
                     严格只输出 JSON：{{\"correct\": true 或 false, \"comment\": \"一句中文点评\"}}。"
                )),
            ],
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
    chat_stream(
        settings,
        vec![
            ChatMessage::system("你是一位耐心的中文学习助教，回答要准确、简洁。"),
            ChatMessage::user(&format!(
                "{prev}学习内容：\n{unit_text}\n\n学生的问题：{question}\n\n请基于学习内容回答问题，用中文。",
                prev = prev_context_section(prev_context)
            )),
        ],
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
    chat_stream(
        settings,
        vec![
            ChatMessage::system("你是一位耐心的中文学习助教，回答要准确、简洁。"),
            ChatMessage::user(&format!(
                "{prev}学习内容：\n{unit_text}\n\n【用户选中的原文】\n{quote}\n\n学生的问题：{question}\n\n请基于学习内容和选中的原文回答问题，用中文。",
                prev = prev_context_section(prev_context)
            )),
        ],
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
        &settings.model,
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
        &settings.model,
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
        }
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
        let (ok, comment) = grade_short(&s, "题干", "参考答案", "太短").await.unwrap();
        assert!(!ok);
        assert_eq!(comment, "mock 点评");
        let (ok, _) = grade_short(
            &s,
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
            assert!(!body.contains("reasoning_effort"), "重试应去掉思考参数：{body}");
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
            assert!(!body.contains("reasoning_effort"), "重试应去掉思考参数：{body}");
            let events = "data: {\"choices\":[{\"delta\":{\"content\":\"答\"}}]}\n\n\
                          data: {\"choices\":[{\"delta\":{\"content\":\"案\"}}]}\n\n\
                          data: [DONE]\n\n";
            write_http(&mut second, "200 OK", "text/event-stream", events);
        });

        let settings = LlmSettings {
            base_url: format!("http://{addr}"),
            api_key: "sk-test".to_string(),
            model: "stream-retry-model".to_string(),
            auto_start_reading: false,
            disable_thinking: true,
        };
        let full = chat_stream(&settings, vec![ChatMessage::user("hi")], &mut |_d| {})
            .await
            .unwrap();
        assert_eq!(full, "答案");
        handle.join().unwrap();
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
            auto_start_reading: false,
            disable_thinking: false,
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

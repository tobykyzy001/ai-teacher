use serde::{Deserialize, Serialize};
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

/// OpenAI 兼容的 chat/completions 调用。base_url 是否以 /v1 结尾均可。
pub async fn chat(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("请先在设置中配置 API Key".to_string());
    }
    let url = normalize_base_url(base_url);
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    let mut retries = 0u32;
    loop {
        let response = client
            .post(&url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await;
        let resp = match response {
            Ok(r) => r,
            Err(e) => return Err(format!("网络错误：{e}")),
        };
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
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
        let parsed: ChatResponse =
            serde_json::from_str(&text).map_err(|e| format!("解析 AI 响应失败：{e}"))?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .filter(|s| !s.trim().is_empty());
        return content.ok_or_else(|| "AI 返回内容为空".to_string());
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

pub async fn explain(settings: &LlmSettings, unit_text: &str) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(MOCK_EXPLAIN.to_string());
    }
    chat(
        &settings.base_url,
        &settings.api_key,
        &settings.model,
        vec![
            ChatMessage::system("你是一位耐心、风趣的中文学习助教，擅长把材料讲得通俗易懂。"),
            ChatMessage::user(&format!(
                "请阅读下面的学习内容，写一篇 Markdown 格式的讲解，必须包含以下三个小节：\n\
                 「## 背景引入」：用一两段话介绍这章内容在讲什么、为什么值得学；\n\
                 「## 重点梳理」：用无序列表列出 3-6 个核心要点；\n\
                 「## 难点解释」：挑出 1-3 个最难理解的概念，用通俗的方式解释。\n\n\
                 学习内容：\n{unit_text}"
            )),
        ],
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

pub async fn make_quiz(
    settings: &LlmSettings,
    unit_text: &str,
    knowledge_points: &[String],
) -> Result<Vec<RawQuizQuestion>, String> {
    let raw = if is_mock_enabled() {
        mock_quiz_json(knowledge_points)
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
                    "请基于下面的学习内容和知识点，出一套共 5 道题的中文测验：3 道单选题和 2 道简答题。\
                     严格只输出一个 JSON 数组，共 5 个对象，每个对象的字段为：\n\
                     qtype：\"mcq\"（单选）或 \"short\"（简答）；\n\
                     stem：题干；\n\
                     options：单选题为恰好 4 个选项（形如 \"A. xxx\"），简答题为空数组；\n\
                     answer：单选题为正确选项字母（如 \"A\"），简答题为参考答案；\n\
                     explanation：答案解析；\n\
                     knowledge_point：题目考查的知识点。\n\
                     不要输出任何其他文字。\n\n\
                     知识点：{kps}\n\n\
                     学习内容：\n{unit_text}"
                )),
            ],
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

fn mock_quiz_json(knowledge_points: &[String]) -> String {
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
    for i in 0..3 {
        let k = kp(i);
        let correct = letters[i];
        questions.push(serde_json::json!({
            "qtype": "mcq",
            "stem": format!("关于「{k}」，下列说法正确的是哪一项？"),
            "options": letters.iter().map(|l| format!("{l}. {k} 的说法 {l}")).collect::<Vec<_>>(),
            "answer": correct,
            "explanation": format!("mock 解析：{correct} 是关于「{k}」最准确的表述。"),
            "knowledge_point": k,
        }));
    }
    for i in 0..2 {
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

pub async fn ask(
    settings: &LlmSettings,
    unit_text: &str,
    question: &str,
) -> Result<String, String> {
    if is_mock_enabled() {
        return Ok(format!("mock 回答：{question}"));
    }
    chat(
        &settings.base_url,
        &settings.api_key,
        &settings.model,
        vec![
            ChatMessage::system("你是一位耐心的中文学习助教，回答要准确、简洁。"),
            ChatMessage::user(&format!(
                "学习内容：\n{unit_text}\n\n学生的问题：{question}\n\n请基于学习内容回答问题，用中文。"
            )),
        ],
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
        }
    }

    #[tokio::test]
    async fn mock_explanation_sections() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let out = explain(&mock_settings(), "任意内容").await.unwrap();
        assert!(out.contains("背景引入"));
        assert!(out.contains("重点梳理"));
        assert!(out.contains("难点解释"));
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
        let raw = make_quiz(&mock_settings(), "内容", &kps).await.unwrap();
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
        let raw = make_quiz(&mock_settings(), "内容", &[]).await.unwrap();
        assert_eq!(raw.len(), 5);
        assert!(normalize_quiz(&raw).is_some());
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
    async fn mock_ask_echoes_question() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let out = ask(&mock_settings(), "内容", "什么是间隔效应？")
            .await
            .unwrap();
        assert_eq!(out, "mock 回答：什么是间隔效应？");
    }

    #[tokio::test]
    async fn mock_ping_ok() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        ping(&mock_settings()).await.unwrap();
    }

    #[tokio::test]
    async fn empty_api_key_rejected_before_any_request() {
        // chat() 不经过 mock 分支，空 key 直接报错，且不会发起网络请求
        let err = chat("", "", "m", vec![ChatMessage::user("hi")])
            .await
            .unwrap_err();
        assert_eq!(err, "请先在设置中配置 API Key");
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

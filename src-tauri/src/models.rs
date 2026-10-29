use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitStatus {
    Unread,
    Reading,
    Done,
    /// 用户手动标记"无需掌握"（✔）：不计做题，进度照常计入，可撤销
    Skipped,
}

impl UnitStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            UnitStatus::Unread => "unread",
            UnitStatus::Reading => "reading",
            UnitStatus::Done => "done",
            UnitStatus::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "unread" => Some(UnitStatus::Unread),
            "reading" => Some(UnitStatus::Reading),
            "done" => Some(UnitStatus::Done),
            "skipped" => Some(UnitStatus::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QType {
    Mcq,
    Short,
}

impl QType {
    pub fn as_str(self) -> &'static str {
        match self {
            QType::Mcq => "mcq",
            QType::Short => "short",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "mcq" | "choice" | "single_choice" | "single" | "单选" | "选择题" => {
                Some(QType::Mcq)
            }
            "short" | "short_answer" | "简答" | "简答题" | "主观题" => Some(QType::Short),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 打开全新章节时是否自动开始 AI 领读；默认关闭（手动点"开始领读"才生成）
    pub auto_start_reading: bool,
    /// 关闭模型思考模式提速：按 OpenAI 兼容协议注入 `reasoning_effort`（none/medium，见 llm::thinking_params）
    pub disable_thinking: bool,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com".to_string(),
            api_key: String::new(),
            model: "deepseek-chat".to_string(),
            auto_start_reading: false,
            disable_thinking: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSummary {
    pub id: i64,
    pub title: String,
    pub format: String,
    pub added_at: String,
    pub total_units: i64,
    pub done_units: i64,
    /// EPUB 内嵌封面提取到 books/<id>/cover.* 的绝对路径；无封面（PDF/MD/TXT）为 None
    #[serde(default)]
    pub cover_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unit {
    pub id: i64,
    pub book_id: i64,
    pub idx: i64,
    pub title: String,
    pub status: UnitStatus,
    pub mastery: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitContent {
    pub unit: Unit,
    pub book_title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingSession {
    pub explanation: String,
    pub knowledge_points: Vec<String>,
}

/// 题目对前端的形态：绝不包含 answer / explanation 字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizQuestion {
    pub id: i64,
    pub unit_id: i64,
    pub qtype: QType,
    pub stem: String,
    pub options: Vec<String>,
    pub knowledge_point: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradeResult {
    pub correct: bool,
    pub reference_answer: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: i64,
    pub unit_id: i64,
    pub unit_title: String,
    pub book_title: String,
    pub knowledge_point: String,
    pub question_id: Option<i64>,
    pub due_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitSuggestion {
    pub book_id: i64,
    pub book_title: String,
    pub unit: Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayPlan {
    pub due_reviews: Vec<ReviewItem>,
    pub next_units: Vec<UnitSuggestion>,
    pub reviewed_today: i64,
    pub due_total: i64,
    pub streak_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewTask {
    pub item: ReviewItem,
    pub question: Option<QuizQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookDetail {
    pub book: BookSummary,
    pub units: Vec<Unit>,
    /// 按章分组的单元视图（与 units 同一数据源，分组规则见 ingest::group_units_into_chapters）
    pub chapters: Vec<ChapterGroup>,
}

/// 书详情页的一章：章内单元按书内顺序排列。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterGroup {
    pub title: String,
    pub first_unit_id: i64,
    pub units: Vec<Unit>,
}

/// 阅读页一章的内容：整章连续展示 + 章间翻页。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterContent {
    pub book_title: String,
    pub chapter_title: String,
    pub chapter_number: i64, // 从 1 开始
    pub chapter_count: i64,
    pub prev_chapter_unit_id: Option<i64>, // 上一章首节的 unit_id，无则 None
    pub next_chapter_unit_id: Option<i64>,
    pub segments: Vec<ChapterSegment>,
    /// 已保存的章领读讲解（生成过才有；重开页面直接展示，不重新生成）
    pub saved_session: Option<ReadingSession>,
    /// 该书图片目录的绝对路径（books/<id>/images）；没有插图的书为 None
    pub images_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSegment {
    pub unit: Unit,
    pub content: String,
}

/// 错题条目：绝不包含 answer / explanation 字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrongQuestion {
    pub question_id: i64,
    pub unit_id: i64,
    pub unit_title: String,
    pub book_id: i64,
    pub book_title: String,
    pub stem: String,
    pub knowledge_point: String,
    pub wrong_count: i64,
    pub last_wrong_at: String,
    pub resolved: bool,
}

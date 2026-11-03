import { Channel, invoke } from "@tauri-apps/api/core";

// ---------- 类型定义（与 Rust 端 serde 序列化出的 JSON 一一对应） ----------

export interface LlmSettings {
  base_url: string;
  api_key: string;
  model: string;
  auto_start_reading: boolean; // 打开全新章节时自动开始 AI 领读；默认 false
  disable_thinking: boolean; // 关闭模型思考模式提速（OpenAI 兼容协议 reasoning_effort）；默认 true
}

export interface ConnectionTestResult {
  ok: boolean;
  message: string;
}

export interface BookSummary {
  id: number;
  title: string;
  format: string; // md | txt | epub | pdf
  added_at: string; // ISO date
  total_units: number;
  done_units: number;
  cover_path?: string | null; // EPUB 内嵌封面提取到 books/<id>/cover.* 的绝对路径
}

export type UnitStatus = "unread" | "reading" | "done" | "skipped";

export interface Unit {
  id: number;
  book_id: number;
  idx: number; // 书内顺序，从 0 开始
  title: string;
  status: UnitStatus;
  mastery: number | null; // 0..1，最近一轮做题正确率
}

export interface UnitContent {
  unit: Unit;
  book_title: string;
  content: string; // 单元正文（纯文本）
}

export interface ReadingSession {
  explanation: string; // AI 领读讲解（Markdown）
  knowledge_points: string[];
}

// ---------- 章节（阅读/领读单位；章由单元标题推导分组） ----------

export interface ChapterGroup {
  title: string;
  first_unit_id: number; // 跳转到该章阅读页的入口 unit_id
  units: Unit[];
}

export interface ChapterSegment {
  unit: Unit;
  content: string; // 该小节正文（纯文本）
}

export interface ChapterContent {
  book_title: string;
  chapter_title: string;
  chapter_number: number; // 从 1 开始
  chapter_count: number;
  prev_chapter_unit_id: number | null; // 上一章首节的 unit_id
  next_chapter_unit_id: number | null;
  segments: ChapterSegment[];
  saved_session: ReadingSession | null; // 已保存的章领读讲解；非空则直接展示不重新生成
  images_dir: string | null; // 该书插图目录的绝对路径；无插图为 null
}

export type QType = "mcq" | "short";

export interface QuizQuestion {
  id: number;
  unit_id: number;
  qtype: QType;
  stem: string;
  options: string[]; // mcq 有 4 个选项；short 为空数组
  knowledge_point: string;
}

export interface GradeResult {
  correct: boolean;
  reference_answer: string;
  explanation: string; // 解析 / 简答题的 AI 点评
}

export interface ReviewItem {
  id: number;
  unit_id: number;
  unit_title: string;
  book_title: string;
  knowledge_point: string;
  question_id: number | null; // 非空表示这是一个"重做错题"的复习
  due_date: string; // YYYY-MM-DD
}

export interface UnitSuggestion {
  book_id: number;
  book_title: string;
  unit: Unit;
}

export interface TodayPlan {
  due_reviews: ReviewItem[];
  next_units: UnitSuggestion[]; // 每本在读的书的下一个未学单元
  reviewed_today: number; // 今日已完成的复习数（答题与自评都算）
  due_total: number; // 今日应复习总数 = reviewed_today + 剩余到期
  streak_days: number; // 连续学习天数
}

export interface ReviewTask {
  item: ReviewItem;
  question: QuizQuestion | null;
}

export interface WrongQuestion {
  question_id: number;
  unit_id: number;
  unit_title: string;
  book_id: number;
  book_title: string;
  stem: string;
  knowledge_point: string;
  wrong_count: number;
  last_wrong_at: string; // ISO 时间
  resolved: boolean; // 重练答对后置 true，永不删除
}

export interface BookDetail {
  book: BookSummary;
  units: Unit[];
  chapters: ChapterGroup[]; // 与 units 同一数据源的章分组视图
}

// ---------- WebDAV 备份/同步 ----------
//
// 语义是**快照覆盖**而非双向合并：每次同步整包替换远端/本地，不做字段级 merge。
// 因此适合"同一时间只在一台设备上学习"，两台设备交叉使用会互相覆盖。

export interface WebDavSettings {
  enabled: boolean; // 自动同步开关：写操作后 debounce 上传进度包
  base_url: string; // 任意标准 WebDAV 端点，如 https://dav.example.com/dav/
  username: string; // 账号（多为邮箱）
  password: string; // 多数服务需用「应用专用密码」，不是登录密码
  folder: string; // 远端子目录；留空用默认 ai-teacher
}

export interface BackupStats {
  books: number;
  units: number;
  attempts: number;
  review_items: number;
  done_units: number;
}

export interface BackupManifest {
  schema_version: number; // 数据版本；高于当前应用则拒绝恢复
  app_version: string;
  created_at: string; // RFC3339
  device: string;
  includes_books: boolean;
  db_stats: BackupStats;
}

export interface RemoteSnapshot {
  name: string;
  size: number;
  modified: string;
  manifest: BackupManifest | null; // 解析失败时为 null（界面上标记"未知快照"）
}

export interface LocalStats {
  stats: BackupStats;
  schema_version: number;
  app_version: string;
}

export interface SyncResult {
  ok: boolean;
  message: string;
  bytes: number;
}

// ---------- invoke 封装 ----------

function errMsg(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

/// 会改库的命令：成功后通知自动同步"该传了"。
/// 只在**写**命令里登记——读命令不触发同步（尤其 list_books 会顺带补建封面写盘，
/// 若按"有 IO 就传"判断，用户只是看了眼书架就会把远端覆盖掉）。
const WRITE_COMMANDS = new Set([
  "save_settings",
  "import_book",
  "remove_book",
  "set_unit_skipped",
  "set_chapter_skipped",
  "start_reading",
  "start_chapter_reading",
  "generate_quiz",
  "regenerate_quiz",
  "submit_answer",
  "submit_review",
]);

type SyncHook = () => void;
let syncHook: SyncHook | null = null;

/// 由自动同步模块注册：写命令成功后调用（内部自行 debounce）。
export function setWriteHook(fn: SyncHook | null): void {
  syncHook = fn;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const out = await invoke<T>(cmd, args);
    if (WRITE_COMMANDS.has(cmd)) syncHook?.();
    return out;
  } catch (e) {
    throw new Error(errMsg(e));
  }
}

// 流式参数：给定 onDelta 时通过 Channel 接收增量文本，否则不传该参数
function streamArg(onDelta?: (s: string) => void): Record<string, unknown> {
  if (!onDelta) return {};
  const ch = new Channel<string>();
  ch.onmessage = onDelta;
  return { onProgress: ch };
}

export const api = {
  // 设置
  getSettings: () => call<LlmSettings>("get_settings"),
  saveSettings: (settings: LlmSettings) => call<void>("save_settings", { settings }),
  testLlmConnection: () => call<ConnectionTestResult>("test_llm_connection"),
  // 取走一条"模型不支持思考参数"的一次性提示（AI 动作结束后调用）
  takeThinkingNotice: () => call<string | null>("take_thinking_notice"),

  // 书架
  importBook: (path: string) => call<BookSummary>("import_book", { path }),
  listBooks: () => call<BookSummary[]>("list_books"),
  removeBook: (bookId: number) => call<void>("remove_book", { bookId }),
  getBookDetail: (bookId: number) => call<BookDetail>("get_book_detail", { bookId }),

  // 今日
  getToday: () => call<TodayPlan>("get_today"),

  // 领读
  getUnitContent: (unitId: number) => call<UnitContent>("get_unit_content", { unitId }),
  startReading: (unitId: number, onDelta?: (s: string) => void) =>
    call<ReadingSession>("start_reading", { unitId, ...streamArg(onDelta) }),
  ask: (unitId: number, question: string, onDelta?: (s: string) => void) =>
    call<string>("ask", { unitId, question, ...streamArg(onDelta) }),
  askSelection: (unitId: number, quote: string, question: string, onDelta?: (s: string) => void) =>
    call<string>("ask_selection", { unitId, quote, question, ...streamArg(onDelta) }),

  // 章节阅读
  getChapterContent: (unitId: number) => call<ChapterContent>("get_chapter_content", { unitId }),
  startChapterReading: (unitId: number, onDelta?: (s: string) => void) =>
    call<ReadingSession>("start_chapter_reading", { unitId, ...streamArg(onDelta) }),
  askChapter: (unitId: number, question: string, onDelta?: (s: string) => void) =>
    call<string>("ask_chapter", { unitId, question, ...streamArg(onDelta) }),
  askChapterSelection: (unitId: number, quote: string, question: string, onDelta?: (s: string) => void) =>
    call<string>("ask_chapter_selection", { unitId, quote, question, ...streamArg(onDelta) }),

  // 无需掌握标记：计入进度、清待复习，可撤销；章级参数传章内任一 unitId（前端用章首节）
  setUnitSkipped: (unitId: number, skipped: boolean) =>
    call<void>("set_unit_skipped", { unitId, skipped }),
  setChapterSkipped: (unitId: number, skipped: boolean) =>
    call<void>("set_chapter_skipped", { unitId, skipped }),

  // 做题
  generateQuiz: (unitId: number) => call<QuizQuestion[]>("generate_quiz", { unitId }),
  // 换一批新题：旧题退役不删除，错题本/复习记录保留
  regenerateQuiz: (unitId: number) => call<QuizQuestion[]>("regenerate_quiz", { unitId }),
  submitAnswer: (questionId: number, givenAnswer: string) =>
    call<GradeResult>("submit_answer", { questionId, givenAnswer }),
  getQuestion: (questionId: number) => call<QuizQuestion>("get_question", { questionId }),

  // 复习
  startReview: (itemId: number) => call<ReviewTask>("start_review", { itemId }),
  submitReview: (itemId: number, passed: boolean) => call<void>("submit_review", { itemId, passed }),

  // 错题本
  listWrongQuestions: (filter: { bookId?: number | null; unitId?: number | null }) =>
    call<WrongQuestion[]>("list_wrong_questions", {
      bookId: filter.bookId ?? null,
      unitId: filter.unitId ?? null,
    }),

  // ---------- WebDAV 备份/同步 ----------
  getSyncSettings: () => call<WebDavSettings>("get_sync_settings"),
  saveSyncSettings: (settings: WebDavSettings) =>
    call<void>("save_sync_settings", { settings }),
  testSyncConnection: (settings: WebDavSettings) =>
    call<ConnectionTestResult>("test_sync_connection", { settings }),
  getSyncLocalStats: () => call<LocalStats>("get_sync_local_stats"),
  // withBooks=true 连 books/ 一起打（体积取决于书）；false 只打进度（随答题量增长）
  syncUpload: (settings: WebDavSettings, withBooks: boolean) =>
    call<SyncResult>("sync_upload", { settings, withBooks }),
  syncListRemote: (settings: WebDavSettings) =>
    call<RemoteSnapshot[]>("sync_list_remote", { settings }),
  syncRestore: (settings: WebDavSettings, key: string, backupCurrent: boolean) =>
    call<SyncResult>("sync_restore", { settings, key, backupCurrent }),
  // 自动同步：有未同步写入时静默上传，返回 null 表示没有要传的
  syncAutoFlush: () => call<SyncResult | null>("sync_auto_flush"),
};

import { Channel, invoke } from "@tauri-apps/api/core";

// ---------- 类型定义（与 Rust 端 serde 序列化出的 JSON 一一对应） ----------

export interface LlmSettings {
  base_url: string;
  api_key: string;
  model: string;
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
}

export type UnitStatus = "unread" | "reading" | "done";

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
}

// ---------- invoke 封装 ----------

function errMsg(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
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

  // 做题
  generateQuiz: (unitId: number) => call<QuizQuestion[]>("generate_quiz", { unitId }),
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
};

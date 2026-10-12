use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::{config_path, ConfigStore};
use crate::db::{Db, QuestionRow, ReviewRow, UnitRow};
use crate::ingest;
use crate::llm;
use crate::models::*;
use crate::scheduler;

pub struct AppState {
    pub db: Db,
    pub app_data_dir: PathBuf,
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn read_unit_content(unit: &UnitRow) -> Result<String, String> {
    std::fs::read_to_string(&unit.content_path).map_err(|e| format!("无法读取单元内容：{e}"))
}

fn question_to_public(q: &QuestionRow) -> QuizQuestion {
    QuizQuestion {
        id: q.id,
        unit_id: q.unit_id,
        qtype: crate::models::QType::parse(&q.qtype).unwrap_or(crate::models::QType::Short),
        stem: q.stem.clone(),
        options: q.options.clone(),
        knowledge_point: q.knowledge_point.clone(),
    }
}

fn row_to_unit(row: &UnitRow) -> Unit {
    Unit {
        id: row.id,
        book_id: row.book_id,
        idx: row.idx,
        title: row.title.clone(),
        status: UnitStatus::parse(&row.status).unwrap_or(UnitStatus::Unread),
        mastery: row.mastery,
    }
}

// ---------- 业务实现（可在无 Tauri 运行时的测试中直接调用） ----------

pub fn load_settings(app_data_dir: &Path) -> LlmSettings {
    ConfigStore::new(config_path(app_data_dir)).load()
}

pub fn save_settings_to(app_data_dir: &Path, settings: &LlmSettings) -> Result<(), String> {
    ConfigStore::new(config_path(app_data_dir)).save(settings)
}

/// 前一单元上下文：优先用已存的单元摘要，否则取上一单元正文前 300 字符。
fn prev_context(db: &Db, unit: &UnitRow) -> Option<String> {
    let prev = db.prev_unit(unit.id).ok()??;
    if let Some(summary) = db.get_unit_summary(prev.id).ok().flatten() {
        return Some(format!("《{}》：{}", prev.title, summary));
    }
    let content = read_unit_content(&prev).ok()?;
    let excerpt: String = content.chars().take(300).collect();
    Some(format!("《{}》：{}", prev.title, excerpt))
}

pub async fn start_reading_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
    on_explain: &mut (dyn FnMut(&str) + Send),
) -> Result<ReadingSession, String> {
    let (unit, _book_title) = db
        .get_unit_with_book_title(unit_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该单元")?;
    let content = read_unit_content(&unit)?;
    let prev_ctx = prev_context(db, &unit);
    let explanation =
        llm::explain_stream(settings, &content, prev_ctx.as_deref(), on_explain).await?;

    ensure_unit_registered(db, settings, &unit, &content).await?;
    // 知识点取已入库的（与复习项一致）；首次学习时即 ensure_unit_registered 提取的
    let knowledge_points = db
        .knowledge_points_of_unit(unit_id)
        .map_err(|e| e.to_string())?;
    Ok(ReadingSession {
        explanation,
        knowledge_points,
    })
}

pub async fn ask_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let unit = db
        .get_unit(unit_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该单元")?;
    let content = read_unit_content(&unit)?;
    let prev_ctx = prev_context(db, &unit);
    llm::ask_stream(settings, &content, prev_ctx.as_deref(), question, on_delta).await
}

pub async fn ask_selection_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
    quote: &str,
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let unit = db
        .get_unit(unit_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该单元")?;
    let content = read_unit_content(&unit)?;
    let prev_ctx = prev_context(db, &unit);
    llm::ask_selection_stream(
        settings,
        &content,
        prev_ctx.as_deref(),
        quote,
        question,
        on_delta,
    )
    .await
}

// ---------- 章节级阅读（章为阅读/领读单位，单元仍是做题/复习单位） ----------

/// 章正文喂给 LLM 的字符上限；超出部分由 cap_llm_input 取头尾拼接。
const MAX_CHAPTER_LLM_CHARS: usize = 15000;

/// 每个单元所属"大节"标题：优先按源文件目录层级重解析（不入库，EPUB 取 NCX/nav 顶层
/// 条目）；源文件缺失、解析失败或单元数对不上时，回退按单元标题推导（去"（N）"后缀）。
fn book_unit_group_titles(db: &Db, book_id: i64, rows: &[UnitRow]) -> Vec<String> {
    let fallback = || -> Vec<String> {
        rows.iter()
            .map(|r| ingest::chapter_group_title(&r.title).to_string())
            .collect()
    };
    let source_path = match db.get_book_source_path(book_id) {
        Ok(Some(p)) => p,
        _ => return fallback(),
    };
    let source = Path::new(&source_path);
    if !source.exists() {
        return fallback();
    }
    let parsed = match ingest::parse_file_grouped(source) {
        Ok(p) => p,
        Err(_) => return fallback(),
    };
    ingest::unit_group_titles(&parsed, rows.len()).unwrap_or_else(fallback)
}

/// 分组输入 (id, idx, 大节标题, 内容字符数)；读不到内容的单元按 0 字计。
fn measure_book_units(db: &Db, book_id: i64, rows: &[UnitRow]) -> Vec<(i64, i64, String, usize)> {
    let titles = book_unit_group_titles(db, book_id, rows);
    rows.iter()
        .zip(titles)
        .map(|(r, group_title)| {
            let n = read_unit_content(r).map(|c| c.chars().count()).unwrap_or(0);
            (r.id, r.idx, group_title, n)
        })
        .collect()
}

struct ChapterScope {
    book_title: String,
    chapter_title: String,
    chapter_number: usize,
    chapter_count: usize,
    prev_chapter_unit_id: Option<i64>,
    next_chapter_unit_id: Option<i64>,
    units: Vec<(UnitRow, String)>,
    images_dir: Option<String>,
}

/// 定位 unit_id 所在章：全书单元按标题推导分组（超长章自动拆段，见 ingest）。
fn resolve_chapter(db: &Db, unit_id: i64) -> Result<ChapterScope, String> {
    let (unit, book_title) = db
        .get_unit_with_book_title(unit_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该单元")?;
    let rows = db.list_unit_rows(unit.book_id).map_err(|e| e.to_string())?;
    let measured = measure_book_units(db, unit.book_id, &rows);
    let groups = ingest::group_units_into_chapters(&measured);
    let pos = groups
        .iter()
        .position(|g| g.unit_ids.contains(&unit_id))
        .ok_or("未找到该单元所属章节")?;
    let group = &groups[pos];
    let mut units = Vec::with_capacity(group.unit_ids.len());
    for uid in &group.unit_ids {
        let row = rows.iter().find(|r| r.id == *uid).ok_or("章节数据异常")?;
        let content = read_unit_content(row)?;
        units.push((row.clone(), content));
    }
    // 图片目录：单元文件在 books/<id>/ 下，插图在 images/ 子目录（不存在即无插图）
    let images_dir = units
        .first()
        .and_then(|(row, _)| Path::new(&row.content_path).parent().map(|p| p.to_path_buf()))
        .map(|p| p.join("images"))
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().to_string());
    Ok(ChapterScope {
        book_title,
        chapter_title: group.title.clone(),
        chapter_number: pos + 1,
        chapter_count: groups.len(),
        prev_chapter_unit_id: pos
            .checked_sub(1)
            .and_then(|p| groups[p].unit_ids.first().copied()),
        next_chapter_unit_id: groups
            .get(pos + 1)
            .and_then(|g| g.unit_ids.first().copied()),
        units,
        images_dir,
    })
}

fn chapter_plain_text(scope: &ChapterScope) -> String {
    scope
        .units
        .iter()
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn get_chapter_content_impl(db: &Db, unit_id: i64) -> Result<ChapterContent, String> {
    let scope = resolve_chapter(db, unit_id)?;
    let first_unit_id = scope.units[0].0.id;
    let saved_session = db
        .get_chapter_explanation(first_unit_id)
        .ok()
        .flatten()
        .map(|(explanation, knowledge_points)| ReadingSession {
            explanation,
            knowledge_points,
        });
    Ok(ChapterContent {
        book_title: scope.book_title,
        chapter_title: scope.chapter_title,
        chapter_number: scope.chapter_number as i64,
        chapter_count: scope.chapter_count as i64,
        prev_chapter_unit_id: scope.prev_chapter_unit_id,
        next_chapter_unit_id: scope.next_chapter_unit_id,
        segments: scope
            .units
            .iter()
            .map(|(row, content)| ChapterSegment {
                unit: row_to_unit(row),
                content: content.clone(),
            })
            .collect(),
        saved_session,
        images_dir: scope.images_dir,
    })
}

/// 单元首次学习的幂等登记（章领读与单元领读共用）：状态→reading（done 不动）、
/// 无复习项才提取知识点建复习项（明天到期）、无摘要才生成摘要。
async fn ensure_unit_registered(
    db: &Db,
    settings: &LlmSettings,
    unit: &UnitRow,
    content: &str,
) -> Result<(), String> {
    if unit.status != UnitStatus::Done.as_str() {
        db.update_unit_progress(unit.id, UnitStatus::Reading.as_str(), unit.mastery)
            .map_err(|e| e.to_string())?;
    }
    if !db.has_review_items(unit.id).map_err(|e| e.to_string())? {
        let knowledge_points = llm::knowledge_points(settings, content).await?;
        let due = scheduler::due_date(today(), 1);
        for kp in &knowledge_points {
            db.insert_review_item(
                unit.id,
                kp,
                None,
                scheduler::INITIAL_EASE,
                1,
                0,
                &due,
                "study",
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if db
        .get_unit_summary(unit.id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        match llm::summarize(settings, content).await {
            Ok(summary) => {
                let updated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                if let Err(e) = db.upsert_unit_summary(unit.id, &summary, &updated_at) {
                    eprintln!("保存单元摘要失败：{e}");
                }
            }
            Err(e) => eprintln!("生成单元摘要失败（忽略）：{e}"),
        }
    }
    Ok(())
}

pub async fn start_chapter_reading_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
    on_explain: &mut (dyn FnMut(&str) + Send),
) -> Result<ReadingSession, String> {
    let scope = resolve_chapter(db, unit_id)?;
    let chapter_text = ingest::cap_llm_input(&chapter_plain_text(&scope), MAX_CHAPTER_LLM_CHARS);
    let prev_ctx = prev_context(db, &scope.units[0].0);
    let explanation = llm::explain_chapter_stream(
        settings,
        &scope.chapter_title,
        &chapter_text,
        prev_ctx.as_deref(),
        on_explain,
    )
    .await?;

    // 章内每个小节幂等登记；单小节失败不拖垮整章领读
    for (unit, content) in &scope.units {
        if let Err(e) = ensure_unit_registered(db, settings, unit, content).await {
            eprintln!("小节「{}」登记失败（忽略）：{e}", unit.title);
        }
    }
    // 展示用知识点 = 各小节已入库知识点的有序去重并集
    let mut all_kps: Vec<String> = Vec::new();
    for (unit, _) in &scope.units {
        match db.knowledge_points_of_unit(unit.id) {
            Ok(kps) => {
                for kp in kps {
                    if !all_kps.contains(&kp) {
                        all_kps.push(kp);
                    }
                }
            }
            Err(e) => eprintln!("读取小节知识点失败：{e}"),
        }
    }
    // 讲解本地持久化：重开页面直接展示，"重新领读"时覆盖
    let updated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let first_unit_id = scope.units[0].0.id;
    if let Err(e) = db.upsert_chapter_explanation(first_unit_id, &explanation, &all_kps, &updated_at)
    {
        eprintln!("保存章节领读讲解失败：{e}");
    }
    Ok(ReadingSession {
        explanation,
        knowledge_points: all_kps,
    })
}

pub async fn ask_chapter_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let scope = resolve_chapter(db, unit_id)?;
    let chapter_text = ingest::cap_llm_input(&chapter_plain_text(&scope), MAX_CHAPTER_LLM_CHARS);
    let prev_ctx = prev_context(db, &scope.units[0].0);
    llm::ask_stream(settings, &chapter_text, prev_ctx.as_deref(), question, on_delta).await
}

pub async fn ask_chapter_selection_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
    quote: &str,
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let scope = resolve_chapter(db, unit_id)?;
    let chapter_text = ingest::cap_llm_input(&chapter_plain_text(&scope), MAX_CHAPTER_LLM_CHARS);
    let prev_ctx = prev_context(db, &scope.units[0].0);
    llm::ask_selection_stream(
        settings,
        &chapter_text,
        prev_ctx.as_deref(),
        quote,
        question,
        on_delta,
    )
    .await
}

pub async fn generate_quiz_impl(
    db: &Db,
    settings: &LlmSettings,
    unit_id: i64,
) -> Result<Vec<QuizQuestion>, String> {
    let existing = db.list_questions(unit_id).map_err(|e| e.to_string())?;
    if !existing.is_empty() {
        return Ok(existing.iter().map(question_to_public).collect());
    }
    let unit = db
        .get_unit(unit_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该单元")?;
    let content = read_unit_content(&unit)?;
    let knowledge_points = db
        .knowledge_points_of_unit(unit_id)
        .map_err(|e| e.to_string())?;

    let mut raw = llm::make_quiz(settings, &content, &knowledge_points).await?;
    let mut drafts = llm::normalize_quiz(&raw);
    if drafts.is_none() {
        // 形状不对时重试一次
        raw = llm::make_quiz(settings, &content, &knowledge_points).await?;
        drafts = llm::normalize_quiz(&raw);
    }
    let drafts = drafts.ok_or("AI 生成的题目格式不正确，请稍后重试")?;

    let mut questions = Vec::with_capacity(drafts.len());
    for d in drafts {
        let id = db
            .insert_question(
                unit_id,
                d.qtype.as_str(),
                &d.stem,
                &d.options,
                &d.answer,
                &d.explanation,
                &d.knowledge_point,
            )
            .map_err(|e| e.to_string())?;
        questions.push(QuizQuestion {
            id,
            unit_id,
            qtype: d.qtype,
            stem: d.stem,
            options: d.options,
            knowledge_point: d.knowledge_point,
        });
    }
    Ok(questions)
}

pub async fn submit_answer_impl(
    db: &Db,
    settings: &LlmSettings,
    question_id: i64,
    given_answer: &str,
) -> Result<GradeResult, String> {
    let question = db
        .get_question(question_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该题目")?;
    let qtype = QType::parse(&question.qtype).ok_or("题目数据异常")?;

    let (correct, explanation) = match qtype {
        QType::Mcq => (
            grade_mcq(given_answer, &question.answer, &question.options),
            question.explanation.clone(),
        ),
        QType::Short => {
            let (ok, comment) =
                llm::grade_short(settings, &question.stem, &question.answer, given_answer).await?;
            (ok, comment)
        }
    };

    let graded_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    db.insert_attempt(question_id, given_answer, correct, &graded_at)
        .map_err(|e| e.to_string())?;
    db.insert_log(&scheduler::due_date(today(), 0), "answer")
        .map_err(|e| e.to_string())?;

    if !correct {
        let due = scheduler::due_date(today(), 1);
        match db
            .find_review_item_by_question(
                question.unit_id,
                &question.knowledge_point,
                Some(question_id),
            )
            .map_err(|e| e.to_string())?
        {
            Some(item_id) => {
                db.reset_review_item_wrong(item_id, &due)
                    .map_err(|e| e.to_string())?;
            }
            None => {
                db.insert_review_item(
                    question.unit_id,
                    &question.knowledge_point,
                    Some(question_id),
                    scheduler::INITIAL_EASE,
                    1,
                    0,
                    &due,
                    "wrong",
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    maybe_complete_unit(db, question.unit_id)?;

    Ok(GradeResult {
        correct,
        reference_answer: question.answer.clone(),
        explanation,
    })
}

/// 本单元所有题目都作答过 → 标记完成并计算掌握度（每题取最近一次作答）。
fn maybe_complete_unit(db: &Db, unit_id: i64) -> Result<(), String> {
    let total = db.count_questions(unit_id).map_err(|e| e.to_string())?;
    if total == 0 {
        return Ok(());
    }
    let latest = db.latest_attempts(unit_id).map_err(|e| e.to_string())?;
    if (latest.len() as i64) < total {
        return Ok(());
    }
    let correct = latest.values().filter(|v| **v).count() as f64;
    let mastery = correct / total as f64;
    db.update_unit_progress(unit_id, UnitStatus::Done.as_str(), Some(mastery))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn leading_letter(s: &str) -> Option<char> {
    s.chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
}

/// 去掉选项开头的 "A." / "A、" 之类的字母前缀。
fn strip_option_prefix(s: &str) -> &str {
    let trimmed = s.trim();
    let mut chars = trimmed.char_indices();
    if let Some((_, first)) = chars.next() {
        if first.is_ascii_alphabetic() {
            if let Some((i, second)) = chars.next() {
                if matches!(
                    second,
                    '.' | '．' | '、' | '）' | ')' | ':' | '：' | ' ' | '　'
                ) {
                    return trimmed[i + second.len_utf8()..].trim_start();
                }
            }
        }
    }
    trimmed
}

fn option_letter_by_text(text: &str, options: &[String]) -> Option<char> {
    let t = text.trim().to_lowercase();
    let bare = strip_option_prefix(text).trim().to_lowercase();
    for opt in options {
        if opt.trim().to_lowercase() == t {
            return leading_letter(opt);
        }
        let opt_bare = strip_option_prefix(opt).trim().to_lowercase();
        if !bare.is_empty() && opt_bare == bare {
            return leading_letter(opt);
        }
    }
    None
}

/// 单选题判分：接受 "A"、"a"、"A. 文本"、完整选项文本等写法（忽略大小写）。
fn grade_mcq(given: &str, answer: &str, options: &[String]) -> bool {
    let given_trim = given.trim();
    let answer_trim = answer.trim();
    if given_trim.is_empty() {
        return false;
    }
    if given_trim.eq_ignore_ascii_case(answer_trim) {
        return true;
    }

    let given_letter = leading_letter(given_trim);
    let answer_letter =
        leading_letter(answer_trim).or_else(|| option_letter_by_text(answer_trim, options));
    if let (Some(gl), Some(al)) = (given_letter, answer_letter) {
        if gl == al {
            return true;
        }
    }

    let given_bare = strip_option_prefix(given_trim).trim().to_lowercase();
    let answer_bare = strip_option_prefix(answer_trim).trim().to_lowercase();
    if !given_bare.is_empty() && given_bare == answer_bare {
        return true;
    }

    if let Some(al) = answer_letter {
        for opt in options {
            if leading_letter(opt) == Some(al) {
                let opt_bare = strip_option_prefix(opt).trim().to_lowercase();
                let opt_full = opt.trim().to_lowercase();
                if (!opt_bare.is_empty() && opt_bare == given_bare)
                    || opt_full == given_trim.to_lowercase()
                {
                    return true;
                }
            }
        }
    }
    false
}

pub fn get_unit_content_impl(db: &Db, unit_id: i64) -> Result<UnitContent, String> {
    let (row, book_title) = db
        .get_unit_with_book_title(unit_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该单元")?;
    let content = read_unit_content(&row)?;
    Ok(UnitContent {
        unit: row_to_unit(&row),
        book_title,
        content,
    })
}

pub fn get_book_detail_impl(db: &Db, book_id: i64) -> Result<BookDetail, String> {
    let book = db
        .get_book(book_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到这本书")?;
    let rows = db.list_unit_rows(book_id).map_err(|e| e.to_string())?;
    let units: Vec<Unit> = rows.iter().map(row_to_unit).collect();
    let done = units
        .iter()
        .filter(|u| u.status == UnitStatus::Done)
        .count() as i64;
    let measured = measure_book_units(db, book_id, &rows);
    let chapters = ingest::group_units_into_chapters(&measured)
        .into_iter()
        .filter_map(|g| {
            let first_unit_id = g.unit_ids.first().copied()?;
            let group_units: Vec<Unit> = g
                .unit_ids
                .iter()
                .filter_map(|id| units.iter().find(|u| u.id == *id).cloned())
                .collect();
            Some(ChapterGroup {
                title: g.title,
                first_unit_id,
                units: group_units,
            })
        })
        .collect();
    Ok(BookDetail {
        book: BookSummary {
            id: book.id,
            title: book.title,
            format: book.format,
            added_at: book.added_at,
            total_units: units.len() as i64,
            done_units: done,
        },
        units,
        chapters,
    })
}

pub fn remove_book_impl(db: &Db, app_data_dir: &Path, book_id: i64) -> Result<(), String> {
    db.delete_book(book_id).map_err(|e| e.to_string())?;
    let dir = app_data_dir.join("books").join(book_id.to_string());
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

pub fn get_today_impl(db: &Db) -> Result<TodayPlan, String> {
    let today = today();
    let today_str = scheduler::due_date(today, 0);
    let due_reviews = db.list_due_reviews(&today_str).map_err(|e| e.to_string())?;
    let next_units = db
        .next_unread_units()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(book_id, book_title, unit)| UnitSuggestion {
            book_id,
            book_title,
            unit,
        })
        .collect();
    let reviewed_today = db
        .count_log_on(&today_str, "review")
        .map_err(|e| e.to_string())?;
    let due_total = reviewed_today + due_reviews.len() as i64;
    let streak = streak_days(db, today)?;
    Ok(TodayPlan {
        due_reviews,
        next_units,
        reviewed_today,
        due_total,
        streak_days: streak,
    })
}

/// 连续学习天数：今天没记录就从昨天起往前数，遇到第一个空档即止。
fn streak_days(db: &Db, today: chrono::NaiveDate) -> Result<i64, String> {
    let logged = |d: chrono::NaiveDate| -> Result<bool, String> {
        db.has_log_on(&scheduler::due_date(d, 0))
            .map_err(|e| e.to_string())
    };
    let mut day = if logged(today)? {
        today
    } else {
        today - chrono::Duration::days(1)
    };
    let mut streak = 0i64;
    for _ in 0..3650 {
        if !logged(day)? {
            break;
        }
        streak += 1;
        day = day - chrono::Duration::days(1);
    }
    Ok(streak)
}

pub fn start_review_impl(db: &Db, item_id: i64) -> Result<ReviewTask, String> {
    let row: ReviewRow = db
        .get_review_item(item_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该复习项")?;
    let question = row
        .question_id
        .and_then(|qid| db.get_question(qid).ok().flatten())
        .as_ref()
        .map(question_to_public);
    Ok(ReviewTask {
        item: row.into(),
        question,
    })
}

pub fn submit_review_impl(db: &Db, item_id: i64, passed: bool) -> Result<(), String> {
    let row = db
        .get_review_item(item_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该复习项")?;
    let (reps, ease, interval) =
        scheduler::next_schedule(row.reps, row.ease, row.interval_days, passed);
    let due = scheduler::due_date(today(), interval);
    db.update_review_schedule(item_id, reps, ease, interval, &due)
        .map_err(|e| e.to_string())?;
    db.insert_log(&scheduler::due_date(today(), 0), "review")
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_question_impl(db: &Db, question_id: i64) -> Result<QuizQuestion, String> {
    let row = db
        .get_question(question_id)
        .map_err(|e| e.to_string())?
        .ok_or("未找到该题目")?;
    Ok(question_to_public(&row))
}

pub fn list_wrong_questions_impl(
    db: &Db,
    book_id: Option<i64>,
    unit_id: Option<i64>,
) -> Result<Vec<WrongQuestion>, String> {
    let rows = db
        .list_wrong_questions(book_id, unit_id)
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(Into::into).collect())
}

// ---------- Tauri 命令 ----------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<LlmSettings, String> {
    Ok(load_settings(&state.app_data_dir))
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: LlmSettings) -> Result<(), String> {
    save_settings_to(&state.app_data_dir, &settings)
}

#[tauri::command]
pub async fn test_llm_connection(
    state: State<'_, AppState>,
) -> Result<ConnectionTestResult, String> {
    let settings = load_settings(&state.app_data_dir);
    Ok(match llm::ping(&settings).await {
        Ok(()) => ConnectionTestResult {
            ok: true,
            message: "连接成功，模型可用".to_string(),
        },
        Err(e) => ConnectionTestResult {
            ok: false,
            message: e,
        },
    })
}

#[tauri::command]
pub fn import_book(state: State<'_, AppState>, path: String) -> Result<BookSummary, String> {
    ingest::import_book(&state.db, &state.app_data_dir, &path)
}

#[tauri::command]
pub fn list_books(state: State<'_, AppState>) -> Result<Vec<BookSummary>, String> {
    state.db.list_books().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_book(state: State<'_, AppState>, book_id: i64) -> Result<(), String> {
    remove_book_impl(&state.db, &state.app_data_dir, book_id)
}

#[tauri::command]
pub fn get_book_detail(state: State<'_, AppState>, book_id: i64) -> Result<BookDetail, String> {
    get_book_detail_impl(&state.db, book_id)
}

#[tauri::command]
pub fn get_today(state: State<'_, AppState>) -> Result<TodayPlan, String> {
    get_today_impl(&state.db)
}

#[tauri::command]
pub fn get_unit_content(state: State<'_, AppState>, unit_id: i64) -> Result<UnitContent, String> {
    get_unit_content_impl(&state.db, unit_id)
}

#[tauri::command]
pub async fn start_reading(
    state: State<'_, AppState>,
    unit_id: i64,
    on_progress: tauri::ipc::Channel<String>,
) -> Result<ReadingSession, String> {
    let settings = load_settings(&state.app_data_dir);
    let mut on_explain = |d: &str| {
        let _ = on_progress.send(d.to_string());
    };
    start_reading_impl(&state.db, &settings, unit_id, &mut on_explain).await
}

#[tauri::command]
pub async fn ask(
    state: State<'_, AppState>,
    unit_id: i64,
    question: String,
    on_progress: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    let settings = load_settings(&state.app_data_dir);
    let mut on_delta = |d: &str| {
        let _ = on_progress.send(d.to_string());
    };
    ask_impl(&state.db, &settings, unit_id, &question, &mut on_delta).await
}

#[tauri::command]
pub async fn ask_selection(
    state: State<'_, AppState>,
    unit_id: i64,
    quote: String,
    question: String,
    on_progress: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    let settings = load_settings(&state.app_data_dir);
    let mut on_delta = |d: &str| {
        let _ = on_progress.send(d.to_string());
    };
    ask_selection_impl(
        &state.db,
        &settings,
        unit_id,
        &quote,
        &question,
        &mut on_delta,
    )
    .await
}

#[tauri::command]
pub fn get_chapter_content(
    state: State<'_, AppState>,
    unit_id: i64,
) -> Result<ChapterContent, String> {
    get_chapter_content_impl(&state.db, unit_id)
}

#[tauri::command]
pub async fn start_chapter_reading(
    state: State<'_, AppState>,
    unit_id: i64,
    on_progress: tauri::ipc::Channel<String>,
) -> Result<ReadingSession, String> {
    let settings = load_settings(&state.app_data_dir);
    let mut on_explain = |d: &str| {
        let _ = on_progress.send(d.to_string());
    };
    start_chapter_reading_impl(&state.db, &settings, unit_id, &mut on_explain).await
}

#[tauri::command]
pub async fn ask_chapter(
    state: State<'_, AppState>,
    unit_id: i64,
    question: String,
    on_progress: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    let settings = load_settings(&state.app_data_dir);
    let mut on_delta = |d: &str| {
        let _ = on_progress.send(d.to_string());
    };
    ask_chapter_impl(&state.db, &settings, unit_id, &question, &mut on_delta).await
}

#[tauri::command]
pub async fn ask_chapter_selection(
    state: State<'_, AppState>,
    unit_id: i64,
    quote: String,
    question: String,
    on_progress: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    let settings = load_settings(&state.app_data_dir);
    let mut on_delta = |d: &str| {
        let _ = on_progress.send(d.to_string());
    };
    ask_chapter_selection_impl(
        &state.db,
        &settings,
        unit_id,
        &quote,
        &question,
        &mut on_delta,
    )
    .await
}

#[tauri::command]
pub async fn generate_quiz(
    state: State<'_, AppState>,
    unit_id: i64,
) -> Result<Vec<QuizQuestion>, String> {
    let settings = load_settings(&state.app_data_dir);
    generate_quiz_impl(&state.db, &settings, unit_id).await
}

#[tauri::command]
pub async fn submit_answer(
    state: State<'_, AppState>,
    question_id: i64,
    given_answer: String,
) -> Result<GradeResult, String> {
    let settings = load_settings(&state.app_data_dir);
    submit_answer_impl(&state.db, &settings, question_id, &given_answer).await
}

#[tauri::command]
pub fn start_review(state: State<'_, AppState>, item_id: i64) -> Result<ReviewTask, String> {
    start_review_impl(&state.db, item_id)
}

#[tauri::command]
pub fn submit_review(state: State<'_, AppState>, item_id: i64, passed: bool) -> Result<(), String> {
    submit_review_impl(&state.db, item_id, passed)
}

#[tauri::command]
pub fn get_question(state: State<'_, AppState>, question_id: i64) -> Result<QuizQuestion, String> {
    get_question_impl(&state.db, question_id)
}

#[tauri::command]
pub fn list_wrong_questions(
    state: State<'_, AppState>,
    book_id: Option<i64>,
    unit_id: Option<i64>,
) -> Result<Vec<WrongQuestion>, String> {
    list_wrong_questions_impl(&state.db, book_id, unit_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ai-teacher-cmd-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 建一本书、一个单元，单元正文写到临时文件。
    fn setup_unit(tag: &str) -> (Db, PathBuf, i64) {
        let dir = temp_dir(tag);
        let db = Db::open_in_memory().unwrap();
        let content = "学习内容：记忆的编码、存储与提取。".repeat(60);
        let file = dir.join("unit_0.txt");
        std::fs::write(&file, &content).unwrap();
        let book_id = db
            .insert_book("测试书", "/tmp/测试书.md", "md", "2026-10-06")
            .unwrap();
        let unit_id = db
            .insert_unit(book_id, 0, "第一章", file.to_string_lossy().as_ref())
            .unwrap();
        (db, dir, unit_id)
    }

    fn opts(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn start_reading_creates_review_items_idempotently() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("reading");
        let settings = LlmSettings::default();

        let session = start_reading_impl(&db, &settings, unit_id, &mut |_: &str| {})
            .await
            .unwrap();
        assert_eq!(session.knowledge_points.len(), 3);
        assert!(session.explanation.contains("背景引入"));

        let unit = db.get_unit(unit_id).unwrap().unwrap();
        assert_eq!(unit.status, "reading");

        let tomorrow = scheduler::due_date(today(), 1);
        let items = db.list_due_reviews(&tomorrow).unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|i| i.question_id.is_none()));
        assert!(items.iter().all(|i| i.unit_title == "第一章"));

        // 再次进入：不重复创建复习项
        let _ = start_reading_impl(&db, &settings, unit_id, &mut |_: &str| {})
            .await
            .unwrap();
        assert_eq!(db.list_due_reviews(&tomorrow).unwrap().len(), 3);
        assert!(db.has_review_items(unit_id).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn start_reading_keeps_done_status() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("reading-done");
        db.update_unit_progress(unit_id, "done", Some(0.9)).unwrap();
        start_reading_impl(&db, &LlmSettings::default(), unit_id, &mut |_: &str| {})
            .await
            .unwrap();
        let unit = db.get_unit(unit_id).unwrap().unwrap();
        assert_eq!(unit.status, "done");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn quiz_flow_wrong_answer_creates_review_and_completes_unit() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("quiz");
        let settings = LlmSettings::default();

        let questions = generate_quiz_impl(&db, &settings, unit_id).await.unwrap();
        assert_eq!(questions.len(), 5);
        let mcq: Vec<&QuizQuestion> = questions.iter().filter(|q| q.qtype == QType::Mcq).collect();
        let short: Vec<&QuizQuestion> = questions
            .iter()
            .filter(|q| q.qtype == QType::Short)
            .collect();
        assert_eq!(mcq.len(), 3);
        assert_eq!(short.len(), 2);
        assert!(mcq.iter().all(|q| q.options.len() == 4));
        // 序列化给前端的题目不包含答案
        for q in &questions {
            let json = serde_json::to_string(q).unwrap();
            assert!(!json.contains("\"answer\""));
            assert!(!json.contains("\"explanation\""));
        }

        // 再次生成：直接返回已入库的题目
        let again = generate_quiz_impl(&db, &settings, unit_id).await.unwrap();
        assert_eq!(again.len(), 5);
        assert_eq!(again[0].id, questions[0].id);

        // 答题：3 对 2 错
        let r1 = submit_answer_impl(&db, &settings, mcq[0].id, "A")
            .await
            .unwrap();
        assert!(r1.correct);
        assert!(!r1.reference_answer.is_empty());
        let r2 = submit_answer_impl(&db, &settings, mcq[1].id, "A")
            .await
            .unwrap();
        assert!(!r2.correct, "第二题正确答案是 B");
        let r3 = submit_answer_impl(&db, &settings, mcq[2].id, "c")
            .await
            .unwrap();
        assert!(r3.correct, "大小写不敏感");
        let r4 = submit_answer_impl(&db, &settings, short[0].id, "不会")
            .await
            .unwrap();
        assert!(!r4.correct);
        assert_eq!(r4.explanation, "mock 点评");

        // 还有 1 题未答，单元未完成（做题本身不改变 unread 状态）
        let unit = db.get_unit(unit_id).unwrap().unwrap();
        assert_eq!(unit.status, "unread");

        let r5 = submit_answer_impl(
            &db,
            &settings,
            short[1].id,
            "这是一段足够长的回答，阐述了知识点的核心含义。",
        )
        .await
        .unwrap();
        assert!(r5.correct);

        // 全部作答 → 完成，掌握度 3/5
        let unit = db.get_unit(unit_id).unwrap().unwrap();
        assert_eq!(unit.status, "done");
        let mastery = unit.mastery.unwrap();
        assert!((mastery - 0.6).abs() < 1e-9, "mastery = {mastery}");

        // 错题生成两条复习项（created_from = wrong），指向对应题目
        let tomorrow = scheduler::due_date(today(), 1);
        let due = db.list_due_reviews(&tomorrow).unwrap();
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|i| i.question_id.is_some()));
        assert!(due.iter().any(|i| i.question_id == Some(mcq[1].id)));
        assert!(due.iter().any(|i| i.question_id == Some(short[0].id)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wrong_answer_upserts_existing_review_item() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("upsert");
        let settings = LlmSettings::default();
        let questions = generate_quiz_impl(&db, &settings, unit_id).await.unwrap();
        let q = &questions[0];

        submit_answer_impl(&db, &settings, q.id, "错误答案")
            .await
            .unwrap();
        let tomorrow = scheduler::due_date(today(), 1);
        assert_eq!(db.list_due_reviews(&tomorrow).unwrap().len(), 1);

        // 推进该复习项，然后再答错一次：应重置同一条而不是新增
        let item_id = db.list_due_reviews(&tomorrow).unwrap()[0].id;
        db.update_review_schedule(item_id, 4, 2.4, 15, "2026-12-01")
            .unwrap();
        submit_answer_impl(&db, &settings, q.id, "再次错误")
            .await
            .unwrap();
        assert_eq!(db.list_due_reviews(&tomorrow).unwrap().len(), 1);
        let row = db.get_review_item(item_id).unwrap().unwrap();
        assert_eq!((row.reps, row.interval_days), (0, 1));
        assert_eq!(row.due_date, tomorrow);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ask_impl_returns_mock_answer() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("ask");
        let mut deltas: Vec<String> = Vec::new();
        let out = ask_impl(
            &db,
            &LlmSettings::default(),
            unit_id,
            "什么是记忆？",
            &mut |d: &str| deltas.push(d.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(out, "mock 回答：什么是记忆？");
        assert!(deltas.len() >= 2, "mock 回答应分多段推送");
        assert_eq!(deltas.concat(), out);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn start_reading_streams_deltas_and_saves_summary() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("streaming");
        let settings = LlmSettings::default();

        let mut deltas: Vec<String> = Vec::new();
        let session = start_reading_impl(&db, &settings, unit_id, &mut |d: &str| {
            deltas.push(d.to_string());
        })
        .await
        .unwrap();
        assert!(deltas.len() >= 2, "mock 讲解应分多段推送");
        assert_eq!(deltas.concat(), session.explanation);
        assert_eq!(
            db.get_unit_summary(unit_id).unwrap().as_deref(),
            Some("mock 摘要")
        );

        // 第二次进入同一单元：不报错，已有摘要不被覆盖
        db.upsert_unit_summary(unit_id, "手工摘要", "2026-10-06 12:00:00")
            .unwrap();
        start_reading_impl(&db, &settings, unit_id, &mut |_: &str| {})
            .await
            .unwrap();
        assert_eq!(
            db.get_unit_summary(unit_id).unwrap().as_deref(),
            Some("手工摘要")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ask_selection_impl_streams_mock_answer() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("selection");
        let mut deltas: Vec<String> = Vec::new();
        let out = ask_selection_impl(
            &db,
            &LlmSettings::default(),
            unit_id,
            "选中的原文句子",
            "这句话什么意思？",
            &mut |d: &str| deltas.push(d.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(out, "mock 选段回答：这句话什么意思？");
        assert!(deltas.len() >= 2, "mock 选段回答应分多段推送");
        assert_eq!(deltas.concat(), out);

        assert!(ask_selection_impl(
            &db,
            &LlmSettings::default(),
            9999,
            "选",
            "问",
            &mut |_: &str| {}
        )
        .await
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn submit_answer_and_review_write_learning_log() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("log");
        let settings = LlmSettings::default();
        let today_str = scheduler::due_date(today(), 0);

        let questions = generate_quiz_impl(&db, &settings, unit_id).await.unwrap();
        submit_answer_impl(&db, &settings, questions[0].id, "A")
            .await
            .unwrap();
        assert_eq!(db.count_log_on(&today_str, "answer").unwrap(), 1);
        assert_eq!(db.count_log_on(&today_str, "review").unwrap(), 0);

        let item_id = db
            .insert_review_item(unit_id, "要点甲", None, 2.5, 1, 0, "2026-10-01", "study")
            .unwrap();
        submit_review_impl(&db, item_id, true).unwrap();
        assert_eq!(db.count_log_on(&today_str, "review").unwrap(), 1);
        assert_eq!(db.count_log_on(&today_str, "answer").unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_plan_stats_and_streak() {
        let (db, dir, unit_id) = setup_unit("today-stats");
        db.insert_review_item(
            unit_id,
            "要点甲",
            None,
            2.5,
            1,
            0,
            &scheduler::due_date(today(), 0),
            "study",
        )
        .unwrap();

        // 无任何学习记录
        let plan = get_today_impl(&db).unwrap();
        assert_eq!(plan.due_reviews.len(), 1);
        assert_eq!(plan.reviewed_today, 0);
        assert_eq!(plan.due_total, 1);
        assert_eq!(plan.streak_days, 0);

        // 今天没记录、昨天有 → streak >= 1；再前一天也有 → 2；前天之前空一天 → 断档
        let d = |n: i64| scheduler::due_date(today() - Duration::days(n), 0);
        db.insert_log(&d(1), "review").unwrap();
        db.insert_log(&d(2), "answer").unwrap();
        db.insert_log(&d(4), "review").unwrap();

        let plan = get_today_impl(&db).unwrap();
        assert_eq!(plan.streak_days, 2, "昨天+前天连续，大前天空档");

        // 今天复习一次：reviewed_today=1、due_total=2、streak 接上今天变 3
        db.insert_log(&scheduler::due_date(today(), 0), "review")
            .unwrap();
        let plan = get_today_impl(&db).unwrap();
        assert_eq!(plan.reviewed_today, 1);
        assert_eq!(plan.due_total, 2);
        assert_eq!(plan.streak_days, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_wrong_questions_end_to_end() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, unit_id) = setup_unit("wrong-questions");
        let settings = LlmSettings::default();
        let book_id = db.get_unit(unit_id).unwrap().unwrap().book_id;

        let questions = generate_quiz_impl(&db, &settings, unit_id).await.unwrap();
        let mcq: Vec<&QuizQuestion> = questions.iter().filter(|q| q.qtype == QType::Mcq).collect();
        // 第二道单选题正确答案是 B，答 A → 错
        submit_answer_impl(&db, &settings, mcq[1].id, "A")
            .await
            .unwrap();

        let wrongs = list_wrong_questions_impl(&db, None, None).unwrap();
        assert_eq!(wrongs.len(), 1);
        assert_eq!(wrongs[0].question_id, mcq[1].id);
        assert_eq!(wrongs[0].wrong_count, 1);
        assert!(!wrongs[0].resolved);
        assert_eq!(wrongs[0].unit_title, "第一章");
        assert_eq!(wrongs[0].book_title, "测试书");
        // 序列化给前端的错题不包含答案
        let json = serde_json::to_string(&wrongs[0]).unwrap();
        assert!(!json.contains("answer"));
        assert!(!json.contains("explanation"));

        // 之后答对 → resolved 翻转，但仍在错题列表；过滤条件同时生效
        submit_answer_impl(&db, &settings, mcq[1].id, "B")
            .await
            .unwrap();
        let wrongs = list_wrong_questions_impl(&db, Some(book_id), Some(unit_id)).unwrap();
        assert_eq!(wrongs.len(), 1);
        assert!(wrongs[0].resolved);

        assert!(list_wrong_questions_impl(&db, Some(9999), None)
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_question_impl_public_shape() {
        let (db, dir, unit_id) = setup_unit("get-question");
        let question_id = db
            .insert_question(
                unit_id,
                "mcq",
                "题干是什么？",
                &opts(&["A. 甲", "B. 乙", "C. 丙", "D. 丁"]),
                "A",
                "秘密解析",
                "要点甲",
            )
            .unwrap();

        let q = get_question_impl(&db, question_id).unwrap();
        assert_eq!(q.id, question_id);
        assert_eq!(q.unit_id, unit_id);
        assert_eq!(q.stem, "题干是什么？");
        assert_eq!(q.options.len(), 4);
        let json = serde_json::to_string(&q).unwrap();
        assert!(!json.contains("answer"));
        assert!(!json.contains("explanation"));
        assert!(!json.contains("秘密"));

        assert!(get_question_impl(&db, 9999).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unit_content_roundtrip() {
        let (db, dir, unit_id) = setup_unit("content");
        let uc = get_unit_content_impl(&db, unit_id).unwrap();
        assert_eq!(uc.book_title, "测试书");
        assert_eq!(uc.unit.id, unit_id);
        assert_eq!(uc.unit.status, UnitStatus::Unread);
        assert!(uc.content.contains("学习内容"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn book_detail_and_remove_book() {
        let dir = temp_dir("detail");
        let db = Db::open_in_memory().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/demo-book.md");
        let summary = ingest::import_book(&db, &dir, fixture.to_string_lossy().as_ref()).unwrap();

        let detail = get_book_detail_impl(&db, summary.id).unwrap();
        assert_eq!(detail.book.title, "demo-book");
        assert_eq!(detail.book.total_units, 3);
        assert_eq!(detail.units.len(), 3);
        assert!(detail.units[0].title.contains("第一章"));

        let book_dir = dir.join("books").join(summary.id.to_string());
        assert!(book_dir.exists());
        remove_book_impl(&db, &dir, summary.id).unwrap();
        assert!(!book_dir.exists());
        assert!(db.list_books().unwrap().is_empty());
        assert!(db.list_units(summary.id).unwrap().is_empty());
        assert!(get_book_detail_impl(&db, summary.id).is_err());
        // 重复删除幂等
        remove_book_impl(&db, &dir, summary.id).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_plan_due_and_next_units() {
        let (db, dir, unit_id) = setup_unit("today");
        // 第二本书：唯一单元已完成 → 不出现在 next_units
        let b2 = db
            .insert_book("书二", "/t/2.md", "md", "2026-10-06")
            .unwrap();
        let u2 = db.insert_unit(b2, 0, "唯一", "/t/2-0.txt").unwrap();
        db.update_unit_progress(u2, "done", Some(1.0)).unwrap();
        // 第三本书：没有任何单元 → 跳过
        db.insert_book("书三", "/t/3.md", "md", "2026-10-06")
            .unwrap();

        let yesterday = scheduler::due_date(today() - Duration::days(1), 0);
        db.insert_review_item(unit_id, "要点甲", None, 2.5, 1, 0, &yesterday, "study")
            .unwrap();
        let future = scheduler::due_date(today(), 10);
        db.insert_review_item(unit_id, "要点乙", None, 2.5, 1, 0, &future, "study")
            .unwrap();

        let plan = get_today_impl(&db).unwrap();
        assert_eq!(plan.due_reviews.len(), 1);
        assert_eq!(plan.due_reviews[0].knowledge_point, "要点甲");
        assert_eq!(plan.next_units.len(), 1);
        assert_eq!(plan.next_units[0].book_title, "测试书");
        assert_eq!(plan.next_units[0].unit.id, unit_id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn review_task_and_sm2_progression() {
        let (db, dir, unit_id) = setup_unit("review");
        let question_id = db
            .insert_question(
                unit_id,
                "mcq",
                "题干是什么？",
                &opts(&["A. 甲", "B.乙", "C. 丙", "D. 丁"]),
                "A",
                "解析内容",
                "要点甲",
            )
            .unwrap();
        let item_id = db
            .insert_review_item(
                unit_id,
                "要点甲",
                Some(question_id),
                2.5,
                1,
                0,
                "2026-10-01",
                "wrong",
            )
            .unwrap();

        let task = start_review_impl(&db, item_id).unwrap();
        assert_eq!(task.item.knowledge_point, "要点甲");
        assert_eq!(task.item.unit_title, "第一章");
        assert_eq!(task.item.book_title, "测试书");
        let question = task.question.unwrap();
        assert_eq!(question.id, question_id);
        assert_eq!(question.options.len(), 4);
        let json = serde_json::to_string(&question).unwrap();
        assert!(!json.contains("answer"));

        assert!(start_review_impl(&db, 9999).is_err());

        submit_review_impl(&db, item_id, true).unwrap();
        let row = db.get_review_item(item_id).unwrap().unwrap();
        assert_eq!((row.reps, row.interval_days), (1, 1));
        assert_eq!(row.due_date, scheduler::due_date(today(), 1));

        submit_review_impl(&db, item_id, true).unwrap();
        let row = db.get_review_item(item_id).unwrap().unwrap();
        assert_eq!((row.reps, row.interval_days), (2, 2));

        submit_review_impl(&db, item_id, false).unwrap();
        let row = db.get_review_item(item_id).unwrap().unwrap();
        assert_eq!((row.reps, row.interval_days), (0, 1));
        assert!((row.ease - (2.5 + 0.02 + 0.02 - 0.32)).abs() < 1e-9);

        assert!(submit_review_impl(&db, 9999, true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_roundtrip_through_paths() {
        let dir = temp_dir("settings");
        save_settings_to(
            &dir,
            &LlmSettings {
                base_url: "https://api.example.com".to_string(),
                api_key: "sk-1".to_string(),
                model: "gpt".to_string(),
                auto_start_reading: false,
            },
        )
        .unwrap();
        let s = load_settings(&dir);
        assert_eq!(s.model, "gpt");
        assert_eq!(s.api_key, "sk-1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcq_grading_variants() {
        let options = vec![
            "A. 苹果是水果".to_string(),
            "B. 香蕉是蔬菜".to_string(),
            "C. 梨是肉类".to_string(),
            "D. 桃是矿物".to_string(),
        ];
        assert!(grade_mcq("A", "A", &options));
        assert!(grade_mcq("a", "A", &options));
        assert!(grade_mcq(" A ", "A", &options));
        assert!(grade_mcq("A. 苹果是水果", "A", &options));
        assert!(grade_mcq("苹果是水果", "A", &options));
        assert!(grade_mcq("B. 香蕉是蔬菜", "B", &options));
        assert!(!grade_mcq("B", "A", &options));
        assert!(!grade_mcq("苹果是水果", "B", &options));
        assert!(!grade_mcq("", "A", &options));
        assert!(!grade_mcq("C. 梨是肉类", "A", &options));
        // 答案本身是完整选项文本时也能判
        assert!(grade_mcq("A", "苹果是水果", &options));
        assert!(grade_mcq("A. 苹果是水果", "苹果是水果", &options));
        assert!(!grade_mcq("B", "苹果是水果", &options));
    }

    #[test]
    fn missing_unit_or_question_errors() {
        let (db, dir, _unit_id) = setup_unit("missing");
        assert!(get_unit_content_impl(&db, 9999).is_err());
        assert!(get_book_detail_impl(&db, 9999).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 建一本两章三节的书：第一章（1）（2）+ 第二章，正文写到临时文件。
    fn setup_chapter_book(tag: &str) -> (Db, PathBuf, Vec<i64>) {
        let dir = temp_dir(tag);
        let db = Db::open_in_memory().unwrap();
        let book_id = db
            .insert_book("章书", "/tmp/章书.md", "md", "2026-10-09")
            .unwrap();
        let specs = [
            ("第一章（1）", "第一章第一节的正文内容。"),
            ("第一章（2）", "第一章第二节的正文内容。"),
            ("第二章", "第二章的全部正文内容。"),
        ];
        let mut ids = Vec::new();
        for (idx, (title, body)) in specs.iter().enumerate() {
            let file = dir.join(format!("unit_{idx}.txt"));
            std::fs::write(&file, body.repeat(80)).unwrap();
            let id = db
                .insert_unit(book_id, idx as i64, title, file.to_string_lossy().as_ref())
                .unwrap();
            ids.push(id);
        }
        (db, dir, ids)
    }

    #[test]
    fn chapter_content_groups_units_and_navigates() {
        let (db, dir, ids) = setup_chapter_book("chapter-content");

        let ch1 = get_chapter_content_impl(&db, ids[0]).unwrap();
        assert_eq!(ch1.book_title, "章书");
        assert_eq!(ch1.chapter_title, "第一章");
        assert_eq!((ch1.chapter_number, ch1.chapter_count), (1, 2));
        assert_eq!(ch1.segments.len(), 2);
        assert!(ch1.segments[0].content.contains("第一节"));
        assert!(ch1.segments[1].content.contains("第二节"));
        assert_eq!(ch1.prev_chapter_unit_id, None);
        assert_eq!(ch1.next_chapter_unit_id, Some(ids[2]));
        assert!(ch1.images_dir.is_none(), "无插图的书应为 None");

        // 章内任一小节都定位到同一章
        let same = get_chapter_content_impl(&db, ids[1]).unwrap();
        assert_eq!(same.chapter_number, 1);
        assert_eq!(same.next_chapter_unit_id, Some(ids[2]));

        let ch2 = get_chapter_content_impl(&db, ids[2]).unwrap();
        assert_eq!(ch2.chapter_title, "第二章");
        assert_eq!((ch2.chapter_number, ch2.chapter_count), (2, 2));
        assert_eq!(ch2.prev_chapter_unit_id, Some(ids[0]));
        assert_eq!(ch2.next_chapter_unit_id, None);

        assert!(get_chapter_content_impl(&db, 9999).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn book_detail_includes_chapter_groups() {
        let (db, dir, ids) = setup_chapter_book("detail-chapters");
        let book_id = db.get_unit(ids[0]).unwrap().unwrap().book_id;
        let detail = get_book_detail_impl(&db, book_id).unwrap();
        assert_eq!(detail.units.len(), 3);
        assert_eq!(detail.chapters.len(), 2);
        assert_eq!(detail.chapters[0].title, "第一章");
        assert_eq!(detail.chapters[0].first_unit_id, ids[0]);
        assert_eq!(detail.chapters[0].units.len(), 2);
        assert_eq!(detail.chapters[0].units[1].id, ids[1]);
        assert_eq!(detail.chapters[1].title, "第二章");
        assert_eq!(detail.chapters[1].units.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写一个带嵌套目录（第1章 > 1.1/1.2，第2章 独立）的最小 EPUB。
    fn write_nested_epub(dir: &Path) -> PathBuf {
        use std::io::Write;
        use zip::write::{SimpleFileOptions, ZipWriter};

        const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
        const OPF: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata><dc:title>层级书</dc:title></metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/s1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c3" href="text/s2.xhtml" media-type="application/xhtml+xml"/>
    <item id="c4" href="text/s3.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/><itemref idref="c4"/>
  </spine>
</package>"#;
        const NCX: &str = r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <docTitle><text>层级书</text></docTitle>
  <navMap>
    <navPoint id="n1"><navLabel><text>第1章 基础</text></navLabel><content src="text/ch1.xhtml"/>
      <navPoint id="n2"><navLabel><text>1.1 感知机</text></navLabel><content src="text/s1.xhtml"/></navPoint>
      <navPoint id="n3"><navLabel><text>1.2 回归</text></navLabel><content src="text/s2.xhtml"/></navPoint>
    </navPoint>
    <navPoint id="n4"><navLabel><text>第2章 硬件</text></navLabel><content src="text/s3.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
        let page = |body: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>页</title></head><body><p>{body}</p></body></html>"#
            )
        };
        let files: Vec<(String, String)> = vec![
            ("META-INF/container.xml".into(), CONTAINER.into()),
            ("OEBPS/content.opf".into(), OPF.into()),
            ("OEBPS/toc.ncx".into(), NCX.into()),
            ("OEBPS/text/ch1.xhtml".into(), page("第1章引言。")),
            ("OEBPS/text/s1.xhtml".into(), page("感知机内容。")),
            ("OEBPS/text/s2.xhtml".into(), page("回归内容。")),
            ("OEBPS/text/s3.xhtml".into(), page("硬件内容。")),
        ];
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut buf);
            let options = SimpleFileOptions::default();
            for (name, content) in &files {
                writer.start_file(name, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        let path = dir.join("nested.epub");
        std::fs::write(&path, buf.into_inner()).unwrap();
        path
    }

    #[test]
    fn chapter_grouping_uses_epub_toc_hierarchy() {
        let dir = temp_dir("epub-hier");
        let db = Db::open_in_memory().unwrap();
        let path = write_nested_epub(&dir);
        let summary = ingest::import_book(&db, &dir, path.to_string_lossy().as_ref()).unwrap();
        assert_eq!(summary.total_units, 4);

        let detail = get_book_detail_impl(&db, summary.id).unwrap();
        assert_eq!(detail.units.len(), 4);
        assert_eq!(detail.chapters.len(), 2);
        assert_eq!(detail.chapters[0].title, "第1章 基础");
        assert_eq!(detail.chapters[0].units.len(), 3);
        assert_eq!(detail.chapters[1].title, "第2章 硬件");
        assert_eq!(detail.chapters[1].units.len(), 1);

        // 从 1.2 小节进入：定位到整个第1章，下一章指向第2章首节
        let sec2 = detail.units[2].id;
        let ch = get_chapter_content_impl(&db, sec2).unwrap();
        assert_eq!(ch.chapter_title, "第1章 基础");
        assert_eq!(ch.segments.len(), 3);
        assert_eq!(ch.prev_chapter_unit_id, None);
        assert_eq!(ch.next_chapter_unit_id, Some(detail.units[3].id));

        // 源文件被移走 → 回退为按标题推导（各节独立成章）
        std::fs::remove_file(&path).unwrap();
        let detail2 = get_book_detail_impl(&db, summary.id).unwrap();
        assert_eq!(detail2.chapters.len(), 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn start_chapter_reading_registers_each_unit_idempotently() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, ids) = setup_chapter_book("chapter-reading");
        let settings = LlmSettings::default();

        let mut deltas: Vec<String> = Vec::new();
        let session = start_chapter_reading_impl(&db, &settings, ids[0], &mut |d: &str| {
            deltas.push(d.to_string());
        })
        .await
        .unwrap();
        assert_eq!(deltas.concat(), session.explanation);
        assert!(session.explanation.contains("本章脉络"));
        // 展示知识点 = 各小节知识点的去重并集（mock 每节产出相同 3 个）
        assert_eq!(session.knowledge_points.len(), 3);

        // 章内两小节都登记：状态 reading、复习项、摘要；下一章不受影响
        let tomorrow = scheduler::due_date(today(), 1);
        for uid in &ids[..2] {
            let u = db.get_unit(*uid).unwrap().unwrap();
            assert_eq!(u.status, "reading");
            assert!(db.has_review_items(*uid).unwrap());
            assert!(db.get_unit_summary(*uid).unwrap().is_some());
        }
        assert_eq!(db.list_due_reviews(&tomorrow).unwrap().len(), 6);
        assert_eq!(db.get_unit(ids[2]).unwrap().unwrap().status, "unread");

        // 再次领读同一章：不重复建复习项
        start_chapter_reading_impl(&db, &settings, ids[1], &mut |_: &str| {})
            .await
            .unwrap();
        assert_eq!(db.list_due_reviews(&tomorrow).unwrap().len(), 6);

        // 讲解已持久化：重取章节内容直接拿到，无需重新生成
        let ch = get_chapter_content_impl(&db, ids[0]).unwrap();
        let saved = ch.saved_session.expect("领读后应保存讲解");
        assert!(saved.explanation.contains("本章脉络"));
        assert_eq!(saved.knowledge_points.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chapter_content_without_reading_has_no_saved_session() {
        let (db, dir, ids) = setup_chapter_book("no-saved");
        let ch = get_chapter_content_impl(&db, ids[0]).unwrap();
        assert!(ch.saved_session.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn start_chapter_reading_keeps_done_unit_status() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, ids) = setup_chapter_book("chapter-done");
        db.update_unit_progress(ids[0], "done", Some(0.8)).unwrap();
        // 从章内第二节领读整章：已完成的节不回退状态
        start_chapter_reading_impl(&db, &LlmSettings::default(), ids[1], &mut |_: &str| {})
            .await
            .unwrap();
        assert_eq!(db.get_unit(ids[0]).unwrap().unwrap().status, "done");
        assert_eq!(db.get_unit(ids[1]).unwrap().unwrap().status, "reading");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ask_chapter_and_selection_stream_mock_answers() {
        std::env::set_var("AI_TEACHER_MOCK_LLM", "1");
        let (db, dir, ids) = setup_chapter_book("chapter-ask");
        let settings = LlmSettings::default();

        let mut deltas: Vec<String> = Vec::new();
        let out = ask_chapter_impl(&db, &settings, ids[0], "这章讲了什么？", &mut |d: &str| {
            deltas.push(d.to_string());
        })
        .await
        .unwrap();
        assert_eq!(out, "mock 回答：这章讲了什么？");
        assert_eq!(deltas.concat(), out);

        let out = ask_chapter_selection_impl(
            &db,
            &settings,
            ids[1],
            "选中的一段原文",
            "什么意思？",
            &mut |_: &str| {},
        )
        .await
        .unwrap();
        assert_eq!(out, "mock 选段回答：什么意思？");

        assert!(ask_chapter_impl(&db, &settings, 9999, "问", &mut |_: &str| {})
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

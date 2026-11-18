//! 只读全文检索与自律目标排程；不依赖云端和 LLM。
use crate::{
    db::{Db, UnitRow},
    models::{DailyGoal, SearchHit, Unit},
};
use std::path::{Path, PathBuf};

/// Unicode 字符偏移与以 ASCII 为主的大小写不敏感匹配。
/// 非 ASCII 字符采用 Rust lowercase 映射；偏移始终落在原字符串的 UTF-8 边界上。
pub fn find_matches(text: &str, query: &str, max: usize) -> Vec<(usize, usize)> {
    if query.trim().is_empty() || max == 0 {
        return Vec::new();
    }
    let qchars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if qchars.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for start in 0..chars.len() {
        let mut cursor = start;
        let mut matched = true;
        for expected in &qchars {
            let Some(c) = chars.get(cursor).copied() else {
                matched = false;
                break;
            };
            // 只接受 1:1 Unicode 大小写映射，避免 ß 等长度变化破坏偏移。
            if char::to_lowercase(c).count() != 1 || char::to_lowercase(c).next() != Some(*expected)
            {
                matched = false;
                break;
            }
            cursor += 1;
        }
        if matched {
            out.push((start, cursor));
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

fn snippet(text: &str, start: usize, end: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let begin = start.saturating_sub(40);
    let stop = (end + 60).min(chars.len());
    let text = chars[begin..stop]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{}{}{}",
        if begin > 0 { "…" } else { "" },
        text,
        if stop < chars.len() { "…" } else { "" }
    )
}

fn one_hit(
    unit: &UnitRow,
    chapter: &str,
    text: &str,
    start: usize,
    end: usize,
    title_search: bool,
) -> SearchHit {
    let chars = text.chars().collect::<Vec<_>>();
    SearchHit {
        unit_id: unit.id,
        unit_title: unit.title.clone(),
        chapter_title: chapter.to_string(),
        quote: chars[start..end].iter().collect(),
        context: snippet(text, start, end),
        start,
        end,
        title_search,
    }
}

fn search_unit(
    row: &UnitRow,
    chapter: &str,
    text: &str,
    query: &str,
    per_unit: usize,
) -> Vec<SearchHit> {
    let mut hits = find_matches(&row.title, query, per_unit)
        .into_iter()
        .map(|(start, end)| one_hit(row, chapter, &row.title, start, end, true))
        .collect::<Vec<_>>();
    if hits.len() < per_unit {
        // 原文偏移只存入 hit.image_name，绝不混入 images_dir。
        for (start, end) in find_matches(text, query, per_unit - hits.len()) {
            hits.push(one_hit(row, chapter, text, start, end, false));
        }
    }
    hits
}

pub fn search_book(db: &Db, book_id: i64, query: &str) -> Result<Vec<SearchHit>, String> {
    if query.trim().is_empty() || query.chars().count() > 100 {
        return Ok(Vec::new());
    }
    let rows = db.list_unit_rows(book_id).map_err(|e| e.to_string())?;
    let chapters = if db
        .has_chapter_overrides(book_id)
        .map_err(|e| e.to_string())?
    {
        db.chapter_titles(book_id).map_err(|e| e.to_string())?
    } else {
        std::collections::HashMap::new()
    };
    let mut hits = Vec::new();
    for row in &rows {
        let path = if Path::new(&row.content_path).exists() {
            PathBuf::from(&row.content_path)
        } else {
            db.data_dir()
                .ok()
                .flatten()
                .and_then(|root| {
                    row.content_path
                        .find("books/")
                        .map(|index| root.join(&row.content_path[index..]))
                })
                .unwrap_or_else(|| PathBuf::from(&row.content_path))
        };
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("无法读取「{}」：{e}", row.title))?;
        let chapter = chapters
            .get(&row.id)
            .cloned()
            .unwrap_or_else(|| crate::ingest::chapter_group_title(&row.title).to_string());
        hits.extend(search_unit(row, &chapter, &text, query, 10));
        if hits.len() >= 100 {
            hits.truncate(100);
            break;
        }
    }
    Ok(hits)
}

/// 只读全文检索与目标排程，调度逻辑与 SM-2 分开。
pub fn daily_target_since(
    remaining_units: i64,
    weeks: i64,
    started: chrono::NaiveDate,
    today: chrono::NaiveDate,
) -> (i64, i64) {
    let elapsed = (today - started).num_days().max(0);
    let days_left = weeks
        .max(1)
        .checked_mul(7)
        .unwrap_or(7)
        .saturating_sub(elapsed)
        .max(1);
    let target = if remaining_units <= 0 {
        0
    } else {
        (remaining_units + days_left - 1) / days_left
    };
    (days_left, target)
}

/// 每本书取接下来 daily_target 个小节。完成情况和 SM-2 的到期量互不限制。
pub fn daily_goals(db: &Db) -> Result<Vec<DailyGoal>, String> {
    let mut result = Vec::new();
    let today = chrono::Local::now().date_naive();
    // 与 list_learning_goals 统计 completed_today 用的日期口径保持一致。
    let today_str = today.format("%Y-%m-%d").to_string();
    for goal in db.list_learning_goals().map_err(|e| e.to_string())? {
        let (days_left, target) = daily_target_since(
            goal.remaining_units,
            goal.weeks,
            chrono::NaiveDate::parse_from_str(&goal.started_at, "%Y-%m-%d").unwrap_or(today),
            today,
        );
        let units = match goal.first_remaining_idx {
            None => Vec::new(),
            Some(_) => {
                // 今天已经开始阅读的小节既计入 completed_today，就不能再排进今天的配额：
                // start_reading 只把 unread 推成 reading，仅靠 status 过滤会把同一节推两遍。
                let started: std::collections::HashSet<i64> = db
                    .goal_completed_units_on(goal.book_id, &today_str)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .collect();
                db.list_unit_rows(goal.book_id)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .filter(|u| u.status != "done" && u.status != "skipped")
                    .filter(|u| !started.contains(&u.id))
                    .take(
                        usize::try_from(target.saturating_sub(goal.completed_today))
                            .unwrap_or(usize::MAX),
                    )
                    .map(|row| Unit {
                        id: row.id,
                        book_id: row.book_id,
                        idx: row.idx,
                        title: row.title,
                        status: crate::models::UnitStatus::parse(&row.status)
                            .unwrap_or(crate::models::UnitStatus::Unread),
                        mastery: row.mastery,
                    })
                    .collect()
            }
        };
        result.push(DailyGoal {
            book_id: goal.book_id,
            book_title: goal.book_title,
            weeks: goal.weeks,
            days_left,
            remaining_units: goal.remaining_units,
            daily_target: target,
            completed_today: goal.completed_today,
            today_units: units,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_offsets_are_unicode_and_capped() {
        let matches = find_matches("前言：Memory memory", "MEMORY", 20);
        assert_eq!(
            &"前言：Memory memory".chars().collect::<Vec<_>>()[matches[0].0..matches[0].1],
            &['M', 'e', 'm', 'o', 'r', 'y'] as &[char]
        );
        assert_eq!(find_matches("零基础", "", 5), Vec::new());
        assert_eq!(find_matches("aaaa", "aa", 2).len(), 2);
    }

    #[test]
    fn schedule_handles_rounding_and_terminal_states() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(daily_target_since(14, 1, date, date), (7, 2));
        assert_eq!(daily_target_since(5, 1, date, date), (7, 1));
        assert_eq!(daily_target_since(0, 1, date, date), (7, 0));
        assert_eq!(
            daily_target_since(14, 1, date, date.succ_opt().unwrap()),
            (6, 3)
        );
        assert_eq!(
            daily_target_since(1, 1, date, date.succ_opt().unwrap()),
            (6, 1)
        );
    }
}

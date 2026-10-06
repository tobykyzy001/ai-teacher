use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{BookSummary, ReviewItem, Unit, UnitStatus, WrongQuestion};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("数据库错误：{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("文件系统错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据库连接暂不可用，请重试")]
    LockPoisoned,
}

pub struct Db {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    source_path TEXT NOT NULL UNIQUE,
    format TEXT NOT NULL,
    added_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS units (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    title TEXT NOT NULL,
    content_path TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unread',
    mastery REAL
);
CREATE TABLE IF NOT EXISTS questions (
    id INTEGER PRIMARY KEY,
    unit_id INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
    qtype TEXT NOT NULL,
    stem TEXT NOT NULL,
    options_json TEXT NOT NULL,
    answer TEXT NOT NULL,
    explanation TEXT NOT NULL,
    knowledge_point TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attempts (
    id INTEGER PRIMARY KEY,
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    given_answer TEXT NOT NULL,
    correct INTEGER NOT NULL,
    graded_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS review_items (
    id INTEGER PRIMARY KEY,
    unit_id INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
    knowledge_point TEXT NOT NULL,
    question_id INTEGER,
    ease REAL NOT NULL DEFAULT 2.5,
    interval_days INTEGER NOT NULL DEFAULT 1,
    reps INTEGER NOT NULL DEFAULT 0,
    due_date TEXT NOT NULL,
    created_from TEXT NOT NULL DEFAULT 'study'
);
CREATE TABLE IF NOT EXISTS unit_summaries (
    unit_id INTEGER PRIMARY KEY REFERENCES units(id) ON DELETE CASCADE,
    summary TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS learning_log (
    id INTEGER PRIMARY KEY,
    logged_date TEXT NOT NULL,
    kind TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_units_book ON units(book_id, idx);
CREATE INDEX IF NOT EXISTS idx_questions_unit ON questions(unit_id);
CREATE INDEX IF NOT EXISTS idx_attempts_question ON attempts(question_id, id);
CREATE INDEX IF NOT EXISTS idx_review_due ON review_items(due_date, id);
CREATE INDEX IF NOT EXISTS idx_review_unit ON review_items(unit_id);
CREATE INDEX IF NOT EXISTS idx_log_date ON learning_log(logged_date);
";

#[derive(Debug, Clone)]
pub struct BookRow {
    pub id: i64,
    pub title: String,
    pub format: String,
    pub added_at: String,
}

#[derive(Debug, Clone)]
pub struct UnitRow {
    pub id: i64,
    pub book_id: i64,
    pub idx: i64,
    pub title: String,
    pub content_path: String,
    pub status: String,
    pub mastery: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct QuestionRow {
    pub id: i64,
    pub unit_id: i64,
    pub qtype: String,
    pub stem: String,
    pub options: Vec<String>,
    pub answer: String,
    pub explanation: String,
    pub knowledge_point: String,
}

#[derive(Debug, Clone)]
pub struct ReviewRow {
    pub id: i64,
    pub unit_id: i64,
    pub knowledge_point: String,
    pub question_id: Option<i64>,
    pub ease: f64,
    pub interval_days: i64,
    pub reps: i64,
    pub due_date: String,
    pub unit_title: String,
    pub book_title: String,
}

impl From<ReviewRow> for ReviewItem {
    fn from(r: ReviewRow) -> Self {
        ReviewItem {
            id: r.id,
            unit_id: r.unit_id,
            unit_title: r.unit_title,
            book_title: r.book_title,
            knowledge_point: r.knowledge_point,
            question_id: r.question_id,
            due_date: r.due_date,
        }
    }
}

/// 错题聚合行：wrong_count 只统计答错次数，resolved 以最近一次作答是否正确为准。
#[derive(Debug, Clone)]
pub struct WrongQuestionRow {
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

impl From<WrongQuestionRow> for WrongQuestion {
    fn from(r: WrongQuestionRow) -> Self {
        WrongQuestion {
            question_id: r.question_id,
            unit_id: r.unit_id,
            unit_title: r.unit_title,
            book_id: r.book_id,
            book_title: r.book_title,
            stem: r.stem,
            knowledge_point: r.knowledge_point,
            wrong_count: r.wrong_count,
            last_wrong_at: r.last_wrong_at,
            resolved: r.resolved,
        }
    }
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let conn = self.conn.lock().map_err(|_| DbError::LockPoisoned)?;
        f(&conn)
    }

    // ---------- books ----------

    pub fn insert_book(
        &self,
        title: &str,
        source_path: &str,
        format: &str,
        added_at: &str,
    ) -> Result<i64, DbError> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO books (title, source_path, format, added_at) VALUES (?1, ?2, ?3, ?4)",
                params![title, source_path, format, added_at],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn find_book_by_source_path(&self, source_path: &str) -> Result<Option<i64>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id FROM books WHERE source_path = ?1",
                params![source_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn get_book(&self, id: i64) -> Result<Option<BookRow>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id, title, format, added_at FROM books WHERE id = ?1",
                params![id],
                |row| {
                    Ok(BookRow {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        format: row.get(2)?,
                        added_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn list_books(&self) -> Result<Vec<BookSummary>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT b.id, b.title, b.format, b.added_at,
                        (SELECT COUNT(*) FROM units u WHERE u.book_id = b.id) AS total_units,
                        (SELECT COUNT(*) FROM units u WHERE u.book_id = b.id AND u.status = 'done') AS done_units
                 FROM books b ORDER BY b.id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(BookSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    format: row.get(2)?,
                    added_at: row.get(3)?,
                    total_units: row.get(4)?,
                    done_units: row.get(5)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn delete_book(&self, id: i64) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute("DELETE FROM books WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    // ---------- units ----------

    pub fn insert_unit(
        &self,
        book_id: i64,
        idx: i64,
        title: &str,
        content_path: &str,
    ) -> Result<i64, DbError> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO units (book_id, idx, title, content_path) VALUES (?1, ?2, ?3, ?4)",
                params![book_id, idx, title, content_path],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn list_units(&self, book_id: i64) -> Result<Vec<Unit>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, book_id, idx, title, status, mastery FROM units
                 WHERE book_id = ?1 ORDER BY idx",
            )?;
            let rows = stmt.query_map(params![book_id], map_unit)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn get_unit(&self, id: i64) -> Result<Option<UnitRow>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id, book_id, idx, title, content_path, status, mastery FROM units WHERE id = ?1",
                params![id],
                map_unit_row,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn get_unit_with_book_title(&self, id: i64) -> Result<Option<(UnitRow, String)>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT u.id, u.book_id, u.idx, u.title, u.content_path, u.status, u.mastery, b.title
                 FROM units u JOIN books b ON b.id = u.book_id WHERE u.id = ?1",
                params![id],
                |row| {
                    let unit = map_unit_row(row)?;
                    let book_title: String = row.get(7)?;
                    Ok((unit, book_title))
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn update_unit_progress(
        &self,
        id: i64,
        status: &str,
        mastery: Option<f64>,
    ) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE units SET status = ?2, mastery = ?3 WHERE id = ?1",
                params![id, status, mastery],
            )?;
            Ok(())
        })
    }

    /// 每本书最低 idx 的未学单元（book_id, book_title, unit）。
    pub fn next_unread_units(&self) -> Result<Vec<(i64, String, Unit)>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT u.id, u.book_id, u.idx, u.title, u.status, u.mastery, b.title
                 FROM units u JOIN books b ON b.id = u.book_id
                 WHERE u.status = 'unread'
                   AND u.idx = (SELECT MIN(idx) FROM units u2 WHERE u2.book_id = u.book_id AND u2.status = 'unread')
                 ORDER BY b.id",
            )?;
            let rows = stmt.query_map([], |row| {
                let unit = map_unit(row)?;
                let book_title: String = row.get(6)?;
                Ok((unit.book_id, book_title, unit))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    // ---------- questions ----------

    pub fn insert_question(
        &self,
        unit_id: i64,
        qtype: &str,
        stem: &str,
        options: &[String],
        answer: &str,
        explanation: &str,
        knowledge_point: &str,
    ) -> Result<i64, DbError> {
        let options_json = serde_json::to_string(options).unwrap_or_else(|_| "[]".to_string());
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO questions (unit_id, qtype, stem, options_json, answer, explanation, knowledge_point)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![unit_id, qtype, stem, options_json, answer, explanation, knowledge_point],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn list_questions(&self, unit_id: i64) -> Result<Vec<QuestionRow>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, unit_id, qtype, stem, options_json, answer, explanation, knowledge_point
                 FROM questions WHERE unit_id = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map(params![unit_id], map_question)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn get_question(&self, id: i64) -> Result<Option<QuestionRow>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id, unit_id, qtype, stem, options_json, answer, explanation, knowledge_point
                 FROM questions WHERE id = ?1",
                params![id],
                map_question,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn count_questions(&self, unit_id: i64) -> Result<i64, DbError> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM questions WHERE unit_id = ?1",
                params![unit_id],
                |row| row.get(0),
            )?;
            Ok(n)
        })
    }

    // ---------- attempts ----------

    pub fn insert_attempt(
        &self,
        question_id: i64,
        given_answer: &str,
        correct: bool,
        graded_at: &str,
    ) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO attempts (question_id, given_answer, correct, graded_at) VALUES (?1, ?2, ?3, ?4)",
                params![question_id, given_answer, correct as i64, graded_at],
            )?;
            Ok(())
        })
    }

    /// 每道题最近一次作答是否正确（同题多次作答以最新一次为准）。
    pub fn latest_attempts(&self, unit_id: i64) -> Result<HashMap<i64, bool>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT a.question_id, a.correct FROM attempts a
                 JOIN questions q ON q.id = a.question_id
                 WHERE q.unit_id = ?1 ORDER BY a.id",
            )?;
            let rows = stmt.query_map(params![unit_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0))
            })?;
            let mut map = HashMap::new();
            for row in rows {
                let (qid, correct) = row?;
                map.insert(qid, correct);
            }
            Ok(map)
        })
    }

    // ---------- review items ----------

    pub fn has_review_items(&self, unit_id: i64) -> Result<bool, DbError> {
        self.with_conn(|c| {
            let exists: i64 = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM review_items WHERE unit_id = ?1)",
                params![unit_id],
                |row| row.get(0),
            )?;
            Ok(exists != 0)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_review_item(
        &self,
        unit_id: i64,
        knowledge_point: &str,
        question_id: Option<i64>,
        ease: f64,
        interval_days: i64,
        reps: i64,
        due_date: &str,
        created_from: &str,
    ) -> Result<i64, DbError> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO review_items (unit_id, knowledge_point, question_id, ease, interval_days, reps, due_date, created_from)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    unit_id,
                    knowledge_point,
                    question_id,
                    ease,
                    interval_days,
                    reps,
                    due_date,
                    created_from
                ],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn find_review_item_by_question(
        &self,
        unit_id: i64,
        knowledge_point: &str,
        question_id: Option<i64>,
    ) -> Result<Option<i64>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id FROM review_items
                 WHERE unit_id = ?1 AND knowledge_point = ?2 AND question_id IS ?3 LIMIT 1",
                params![unit_id, knowledge_point, question_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    /// 答错后重置该复习项：reps 归零、间隔 1 天、明天到期。
    pub fn reset_review_item_wrong(&self, id: i64, due_date: &str) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE review_items SET reps = 0, interval_days = 1, due_date = ?2, created_from = 'wrong' WHERE id = ?1",
                params![id, due_date],
            )?;
            Ok(())
        })
    }

    pub fn update_review_schedule(
        &self,
        id: i64,
        reps: i64,
        ease: f64,
        interval_days: i64,
        due_date: &str,
    ) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE review_items SET reps = ?2, ease = ?3, interval_days = ?4, due_date = ?5 WHERE id = ?1",
                params![id, reps, ease, interval_days, due_date],
            )?;
            Ok(())
        })
    }

    pub fn list_due_reviews(&self, today: &str) -> Result<Vec<ReviewItem>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT r.id, r.unit_id, u.title, b.title, r.knowledge_point, r.question_id, r.due_date
                 FROM review_items r
                 JOIN units u ON u.id = r.unit_id
                 JOIN books b ON b.id = u.book_id
                 WHERE r.due_date <= ?1
                 ORDER BY r.due_date, r.id",
            )?;
            let rows = stmt.query_map(params![today], |row| {
                Ok(ReviewItem {
                    id: row.get(0)?,
                    unit_id: row.get(1)?,
                    unit_title: row.get(2)?,
                    book_title: row.get(3)?,
                    knowledge_point: row.get(4)?,
                    question_id: row.get(5)?,
                    due_date: row.get(6)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn get_review_item(&self, id: i64) -> Result<Option<ReviewRow>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT r.id, r.unit_id, r.knowledge_point, r.question_id, r.ease, r.interval_days,
                        r.reps, r.due_date, u.title, b.title
                 FROM review_items r
                 JOIN units u ON u.id = r.unit_id
                 JOIN books b ON b.id = u.book_id
                 WHERE r.id = ?1",
                params![id],
                |row| {
                    Ok(ReviewRow {
                        id: row.get(0)?,
                        unit_id: row.get(1)?,
                        knowledge_point: row.get(2)?,
                        question_id: row.get(3)?,
                        ease: row.get(4)?,
                        interval_days: row.get(5)?,
                        reps: row.get(6)?,
                        due_date: row.get(7)?,
                        unit_title: row.get(8)?,
                        book_title: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn knowledge_points_of_unit(&self, unit_id: i64) -> Result<Vec<String>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT DISTINCT knowledge_point FROM review_items
                 WHERE unit_id = ?1 ORDER BY knowledge_point",
            )?;
            let rows = stmt.query_map(params![unit_id], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    // ---------- unit summaries ----------

    pub fn upsert_unit_summary(
        &self,
        unit_id: i64,
        summary: &str,
        updated_at: &str,
    ) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO unit_summaries (unit_id, summary, updated_at) VALUES (?1, ?2, ?3)",
                params![unit_id, summary, updated_at],
            )?;
            Ok(())
        })
    }

    pub fn get_unit_summary(&self, unit_id: i64) -> Result<Option<String>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT summary FROM unit_summaries WHERE unit_id = ?1",
                params![unit_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    /// 同一本书中 idx 小于当前单元的最大者（上一单元）。
    pub fn prev_unit(&self, unit_id: i64) -> Result<Option<UnitRow>, DbError> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT id, book_id, idx, title, content_path, status, mastery FROM units
                 WHERE book_id = (SELECT book_id FROM units WHERE id = ?1)
                   AND idx < (SELECT idx FROM units WHERE id = ?1)
                 ORDER BY idx DESC LIMIT 1",
                params![unit_id],
                map_unit_row,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    // ---------- learning log ----------

    pub fn insert_log(&self, date: &str, kind: &str) -> Result<(), DbError> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO learning_log (logged_date, kind) VALUES (?1, ?2)",
                params![date, kind],
            )?;
            Ok(())
        })
    }

    pub fn count_log_on(&self, date: &str, kind: &str) -> Result<i64, DbError> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM learning_log WHERE logged_date = ?1 AND kind = ?2",
                params![date, kind],
                |row| row.get(0),
            )?;
            Ok(n)
        })
    }

    pub fn has_log_on(&self, date: &str) -> Result<bool, DbError> {
        self.with_conn(|c| {
            let exists: i64 = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM learning_log WHERE logged_date = ?1)",
                params![date],
                |row| row.get(0),
            )?;
            Ok(exists != 0)
        })
    }

    // ---------- wrong questions ----------

    /// 至少答错过的题目，按最近一次答错时间倒序；resolved = 最近一次作答是否正确。
    pub fn list_wrong_questions(
        &self,
        book_id: Option<i64>,
        unit_id: Option<i64>,
    ) -> Result<Vec<WrongQuestionRow>, DbError> {
        let wrong_count_subquery =
            "(SELECT COUNT(*) FROM attempts w WHERE w.question_id = q.id AND w.correct = 0)";
        let mut sql = format!(
            "SELECT q.id, u.id, u.title, b.id, b.title, q.stem, q.knowledge_point,
                    {wrong_count_subquery} AS wrong_count,
                    (SELECT MAX(w2.graded_at) FROM attempts w2 WHERE w2.question_id = q.id AND w2.correct = 0) AS last_wrong_at,
                    COALESCE((SELECT a3.correct FROM attempts a3 WHERE a3.question_id = q.id ORDER BY a3.id DESC LIMIT 1), 0) AS latest_correct
             FROM questions q
             JOIN units u ON q.unit_id = u.id
             JOIN books b ON u.book_id = b.id"
        );
        let mut clauses = vec![format!("{wrong_count_subquery} > 0")];
        let mut values: Vec<Option<i64>> = Vec::new();
        if let Some(b) = book_id {
            clauses.push(format!("b.id = ?{}", values.len() + 1));
            values.push(Some(b));
        }
        if let Some(u) = unit_id {
            clauses.push(format!("u.id = ?{}", values.len() + 1));
            values.push(Some(u));
        }
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
        sql.push_str(" ORDER BY last_wrong_at DESC, q.id DESC");

        self.with_conn(move |c| {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), |row| {
                Ok(WrongQuestionRow {
                    question_id: row.get(0)?,
                    unit_id: row.get(1)?,
                    unit_title: row.get(2)?,
                    book_id: row.get(3)?,
                    book_title: row.get(4)?,
                    stem: row.get(5)?,
                    knowledge_point: row.get(6)?,
                    wrong_count: row.get(7)?,
                    last_wrong_at: row.get(8)?,
                    resolved: row.get::<_, i64>(9)? != 0,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }
}

fn map_unit(row: &rusqlite::Row<'_>) -> rusqlite::Result<Unit> {
    let status: String = row.get(4)?;
    Ok(Unit {
        id: row.get(0)?,
        book_id: row.get(1)?,
        idx: row.get(2)?,
        title: row.get(3)?,
        status: UnitStatus::parse(&status).unwrap_or(UnitStatus::Unread),
        mastery: row.get(5)?,
    })
}

fn map_unit_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnitRow> {
    Ok(UnitRow {
        id: row.get(0)?,
        book_id: row.get(1)?,
        idx: row.get(2)?,
        title: row.get(3)?,
        content_path: row.get(4)?,
        status: row.get(5)?,
        mastery: row.get(6)?,
    })
}

fn map_question(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuestionRow> {
    let options_json: String = row.get(4)?;
    Ok(QuestionRow {
        id: row.get(0)?,
        unit_id: row.get(1)?,
        qtype: row.get(2)?,
        stem: row.get(3)?,
        options: serde_json::from_str(&options_json).unwrap_or_default(),
        answer: row.get(5)?,
        explanation: row.get(6)?,
        knowledge_point: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn books_with_unit_counts() {
        let db = Db::open_in_memory().unwrap();
        let b1 = db
            .insert_book("书一", "/x/a.md", "md", "2026-10-01")
            .unwrap();
        let b2 = db
            .insert_book("书二", "/x/b.md", "md", "2026-10-02")
            .unwrap();
        let u1 = db.insert_unit(b1, 0, "单元一", "/c/u0.txt").unwrap();
        let u2 = db.insert_unit(b1, 1, "单元二", "/c/u1.txt").unwrap();
        db.insert_unit(b2, 0, "单元三", "/c/u2.txt").unwrap();
        db.update_unit_progress(u1, "done", Some(0.8)).unwrap();
        db.update_unit_progress(u2, "reading", None).unwrap();

        let books = db.list_books().unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!((books[0].total_units, books[0].done_units), (2, 1));
        assert_eq!((books[1].total_units, books[1].done_units), (1, 0));

        let units = db.list_units(b1).unwrap();
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].status, UnitStatus::Done);
        assert_eq!(units[0].mastery, Some(0.8));
        assert_eq!(units[1].status, UnitStatus::Reading);

        assert!(db.find_book_by_source_path("/x/a.md").unwrap().is_some());
        assert!(db.find_book_by_source_path("/x/none.md").unwrap().is_none());
    }

    #[test]
    fn due_reviews_are_joined_and_filtered() {
        let db = Db::open_in_memory().unwrap();
        let b = db
            .insert_book("测试书", "/x/t.md", "md", "2026-10-01")
            .unwrap();
        let u = db.insert_unit(b, 0, "第一章", "/c/u0.txt").unwrap();
        db.insert_review_item(u, "要点一", None, 2.5, 1, 0, "2026-10-05", "study")
            .unwrap();
        db.insert_review_item(u, "要点二", None, 2.5, 1, 0, "2026-10-06", "study")
            .unwrap();
        db.insert_review_item(u, "要点三", None, 2.5, 1, 0, "2026-10-07", "study")
            .unwrap();

        let due = db.list_due_reviews("2026-10-06").unwrap();
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].knowledge_point, "要点一");
        assert_eq!(due[1].knowledge_point, "要点二");
        assert_eq!(due[0].unit_title, "第一章");
        assert_eq!(due[0].book_title, "测试书");

        let all = db.list_due_reviews("2026-10-07").unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn latest_attempt_wins() {
        let db = Db::open_in_memory().unwrap();
        let b = db.insert_book("书", "/x/q.md", "md", "2026-10-01").unwrap();
        let u = db.insert_unit(b, 0, "第一章", "/c/u0.txt").unwrap();
        let q1 = db
            .insert_question(
                u,
                "mcq",
                "题一",
                &opts(&["A. 甲", "B. 乙", "C. 丙", "D. 丁"]),
                "A",
                "解析",
                "要点",
            )
            .unwrap();
        let q2 = db
            .insert_question(u, "short", "题二", &[], "参考", "解析", "要点")
            .unwrap();

        db.insert_attempt(q1, "A", true, "t1").unwrap();
        db.insert_attempt(q1, "B", false, "t2").unwrap();
        db.insert_attempt(q2, "答", true, "t3").unwrap();

        let latest = db.latest_attempts(u).unwrap();
        assert_eq!(latest.len(), 2);
        assert_eq!(latest[&q1], false);
        assert_eq!(latest[&q2], true);
        assert_eq!(db.count_questions(u).unwrap(), 2);
    }

    #[test]
    fn delete_book_cascades() {
        let db = Db::open_in_memory().unwrap();
        let b = db.insert_book("书", "/x/c.md", "md", "2026-10-01").unwrap();
        let u = db.insert_unit(b, 0, "第一章", "/c/u0.txt").unwrap();
        let q = db
            .insert_question(
                u,
                "mcq",
                "题",
                &opts(&["A", "B", "C", "D"]),
                "A",
                "解",
                "点",
            )
            .unwrap();
        db.insert_attempt(q, "A", true, "t").unwrap();
        db.insert_review_item(u, "点", None, 2.5, 1, 0, "2026-10-05", "study")
            .unwrap();

        db.delete_book(b).unwrap();
        assert!(db.list_books().unwrap().is_empty());
        assert!(db.list_units(b).unwrap().is_empty());
        assert!(db.list_questions(u).unwrap().is_empty());
        assert!(db.get_question(q).unwrap().is_none());
        assert_eq!(db.latest_attempts(u).unwrap().len(), 0);
        assert!(db.list_due_reviews("2026-12-31").unwrap().is_empty());
    }

    #[test]
    fn review_item_upsert_lookup() {
        let db = Db::open_in_memory().unwrap();
        let b = db.insert_book("书", "/x/r.md", "md", "2026-10-01").unwrap();
        let u = db.insert_unit(b, 0, "第一章", "/c/u0.txt").unwrap();
        let q = db
            .insert_question(
                u,
                "mcq",
                "题",
                &opts(&["A", "B", "C", "D"]),
                "A",
                "解",
                "要点",
            )
            .unwrap();

        assert!(db
            .find_review_item_by_question(u, "要点", Some(q))
            .unwrap()
            .is_none());
        let id = db
            .insert_review_item(u, "要点", Some(q), 2.5, 3, 2, "2026-10-05", "wrong")
            .unwrap();
        assert_eq!(
            db.find_review_item_by_question(u, "要点", Some(q)).unwrap(),
            Some(id)
        );
        assert!(db
            .find_review_item_by_question(u, "要点", None)
            .unwrap()
            .is_none());

        db.reset_review_item_wrong(id, "2026-10-08").unwrap();
        let row = db.get_review_item(id).unwrap().unwrap();
        assert_eq!(
            (row.reps, row.interval_days, row.due_date.as_str()),
            (0, 1, "2026-10-08")
        );
        let created_from: String = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT created_from FROM review_items WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(created_from, "wrong");
        assert_eq!(row.unit_title, "第一章");

        db.update_review_schedule(id, 3, 2.2, 4, "2026-10-12")
            .unwrap();
        let row = db.get_review_item(id).unwrap().unwrap();
        assert_eq!((row.reps, row.interval_days), (3, 4));
        assert!((row.ease - 2.2).abs() < 1e-9);

        assert!(db.has_review_items(u).unwrap());
        let kps = db.knowledge_points_of_unit(u).unwrap();
        assert_eq!(kps, vec!["要点".to_string()]);
    }

    #[test]
    fn unit_summary_upsert_get_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let b = db
            .insert_book("书", "/x/sum.md", "md", "2026-10-01")
            .unwrap();
        let u = db.insert_unit(b, 0, "第一章", "/c/sum0.txt").unwrap();

        assert!(db.get_unit_summary(u).unwrap().is_none());
        db.upsert_unit_summary(u, "第一版摘要", "2026-10-01 08:00:00")
            .unwrap();
        assert_eq!(
            db.get_unit_summary(u).unwrap().as_deref(),
            Some("第一版摘要")
        );
        db.upsert_unit_summary(u, "第二版摘要", "2026-10-02 08:00:00")
            .unwrap();
        assert_eq!(
            db.get_unit_summary(u).unwrap().as_deref(),
            Some("第二版摘要")
        );

        db.delete_book(b).unwrap();
        assert!(db.get_unit_summary(u).unwrap().is_none());
    }

    #[test]
    fn prev_unit_returns_same_book_neighbor() {
        let db = Db::open_in_memory().unwrap();
        let b1 = db
            .insert_book("书一", "/x/p1.md", "md", "2026-10-01")
            .unwrap();
        let b2 = db
            .insert_book("书二", "/x/p2.md", "md", "2026-10-01")
            .unwrap();
        let u0 = db.insert_unit(b1, 0, "第一章", "/c/p0.txt").unwrap();
        let u1 = db.insert_unit(b1, 1, "第二章", "/c/p1.txt").unwrap();
        let u2 = db.insert_unit(b1, 2, "第三章", "/c/p2.txt").unwrap();
        let other = db.insert_unit(b2, 5, "另一本书", "/c/p3.txt").unwrap();

        let prev = db.prev_unit(u2).unwrap().unwrap();
        assert_eq!(prev.id, u1);
        assert_eq!(prev.title, "第二章");
        assert_eq!(
            db.prev_unit(u1).unwrap().map(|u| u.id),
            Some(u0),
            "上一单元应取 idx 最大的邻居"
        );
        assert!(db.prev_unit(u0).unwrap().is_none());
        // 不跨书：另一本书里 idx 更大的单元不算
        assert!(db.prev_unit(other).unwrap().is_none());
        assert!(db.prev_unit(9999).unwrap().is_none());
    }

    #[test]
    fn learning_log_count_and_exists() {
        let db = Db::open_in_memory().unwrap();
        assert!(!db.has_log_on("2026-10-06").unwrap());
        db.insert_log("2026-10-06", "review").unwrap();
        db.insert_log("2026-10-06", "review").unwrap();
        db.insert_log("2026-10-06", "answer").unwrap();

        assert_eq!(db.count_log_on("2026-10-06", "review").unwrap(), 2);
        assert_eq!(db.count_log_on("2026-10-06", "answer").unwrap(), 1);
        assert_eq!(db.count_log_on("2026-10-05", "review").unwrap(), 0);
        assert!(db.has_log_on("2026-10-06").unwrap());
        assert!(!db.has_log_on("2026-10-05").unwrap());
    }

    #[test]
    fn wrong_question_aggregation() {
        let db = Db::open_in_memory().unwrap();
        let b1 = db
            .insert_book("书一", "/x/w1.md", "md", "2026-10-01")
            .unwrap();
        let u1 = db.insert_unit(b1, 0, "第一章", "/c/w0.txt").unwrap();
        let u2 = db.insert_unit(b1, 1, "第二章", "/c/w1.txt").unwrap();
        let b2 = db
            .insert_book("书二", "/x/w2.md", "md", "2026-10-01")
            .unwrap();
        let u3 = db.insert_unit(b2, 0, "另一章", "/c/w2.txt").unwrap();

        let q1 = db
            .insert_question(
                u1,
                "mcq",
                "题一",
                &opts(&["A. 甲", "B. 乙", "C. 丙", "D. 丁"]),
                "A",
                "解析",
                "要点一",
            )
            .unwrap();
        let q2 = db
            .insert_question(u2, "short", "题二", &[], "参考", "解析", "要点二")
            .unwrap();
        let q3 = db
            .insert_question(
                u3,
                "mcq",
                "题三",
                &opts(&["A. 甲", "B. 乙", "C. 丙", "D. 丁"]),
                "A",
                "解析",
                "要点三",
            )
            .unwrap();
        let q4 = db
            .insert_question(
                u1,
                "mcq",
                "题四",
                &opts(&["A. 甲", "B. 乙", "C. 丙", "D. 丁"]),
                "A",
                "解析",
                "要点四",
            )
            .unwrap();

        // q1：错两次后答对 → resolved，wrong_count 仍为 2
        db.insert_attempt(q1, "B", false, "2026-10-01 10:00:00")
            .unwrap();
        db.insert_attempt(q1, "C", false, "2026-10-02 10:00:00")
            .unwrap();
        db.insert_attempt(q1, "A", true, "2026-10-03 10:00:00")
            .unwrap();
        // q2：只答错一次 → 未解决
        db.insert_attempt(q2, "答错", false, "2026-10-04 10:00:00")
            .unwrap();
        // q3：另一本书的错题，时间最近
        db.insert_attempt(q3, "B", false, "2026-10-05 10:00:00")
            .unwrap();
        // q4：只答对过 → 不在错题列表
        db.insert_attempt(q4, "A", true, "2026-10-06 10:00:00")
            .unwrap();

        let all = db.list_wrong_questions(None, None).unwrap();
        assert_eq!(
            all.iter().map(|w| w.question_id).collect::<Vec<_>>(),
            vec![q3, q2, q1],
            "按最近答错时间倒序"
        );
        let w1 = all.iter().find(|w| w.question_id == q1).unwrap();
        assert_eq!(w1.wrong_count, 2);
        assert_eq!(w1.last_wrong_at, "2026-10-02 10:00:00");
        assert!(w1.resolved);
        assert_eq!(w1.unit_id, u1);
        assert_eq!(w1.unit_title, "第一章");
        assert_eq!(w1.book_id, b1);
        assert_eq!(w1.book_title, "书一");
        assert_eq!(w1.knowledge_point, "要点一");
        assert_eq!(w1.stem, "题一");
        let w2 = all.iter().find(|w| w.question_id == q2).unwrap();
        assert!(!w2.resolved);
        assert_eq!(w2.wrong_count, 1);

        // 过滤：按书 / 按单元 / 两者同时
        let in_b1 = db.list_wrong_questions(Some(b1), None).unwrap();
        assert_eq!(in_b1.len(), 2);
        assert!(in_b1.iter().all(|w| w.book_id == b1));
        let in_u1 = db.list_wrong_questions(None, Some(u1)).unwrap();
        assert_eq!(in_u1.len(), 1);
        assert_eq!(in_u1[0].question_id, q1);
        let narrowed = db.list_wrong_questions(Some(b1), Some(u2)).unwrap();
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].question_id, q2);
        assert!(db
            .list_wrong_questions(Some(9999), None)
            .unwrap()
            .is_empty());

        // 后续答对把 resolved 翻转为 true（仍在错题列表中）
        db.insert_attempt(q3, "A", true, "2026-10-07 10:00:00")
            .unwrap();
        let w3 = db.list_wrong_questions(Some(b2), None).unwrap();
        assert_eq!(w3.len(), 1);
        assert!(w3[0].resolved);
        assert_eq!(w3[0].wrong_count, 1);
    }
}

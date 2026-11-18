//! 单个未学习小节的粒度调整、撤销，以及手动章归属快照。
//! DB 与文件分别提交：事务失败时必须按文件快照回滚，不能只回滚数据库。
use super::*;

/// 只搬运原文，不改写正文。切段保留自己的换行，不能 trim 段内缩进或拆掉 Markdown 标记。
pub fn split_text(text: &str, title: &str, target: usize) -> Vec<ResplitPart> {
    if !(300..=5000).contains(&target) || text.trim().is_empty() {
        return Vec::new();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    for paragraph in text.trim().split_inclusive("\n\n") {
        if paragraph.chars().count() > target {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            parts.extend(split_long_paragraph(paragraph, target));
            continue;
        }
        if !current.is_empty() && current.chars().count() + paragraph.chars().count() > target {
            parts.push(std::mem::take(&mut current));
        }
        current.push_str(paragraph);
        if current.chars().count() >= target {
            parts.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts.retain(|part| !part.trim().is_empty());
    if parts.len() > 1
        && parts
            .last()
            .is_some_and(|part| part.chars().count() < target / 3)
    {
        let tail = parts.pop().unwrap();
        parts.last_mut().unwrap().push_str(&tail);
    }
    if parts.len() < 2 || parts.len() > 100 {
        return Vec::new();
    }
    parts
        .into_iter()
        .enumerate()
        .map(|(index, content)| ResplitPart {
            source_unit_id: None,
            title: format!("{title}（{}）", index + 1),
            content,
            protected: false,
        })
        .collect()
}

fn split_long_paragraph(text: &str, target: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut start = 0;
    while chars.len() - start > target {
        let ideal = start + target;
        let end = (start + target * 4 / 5..(ideal + target / 5).min(chars.len()))
            .filter(|i| chars[*i] == '\n')
            .min_by_key(|i| i.abs_diff(ideal))
            .map(|i| i + 1) // 换行留给当前段，绝不能因此丢掉段间空白
            .unwrap_or(ideal);
        result.push(chars[start..end].iter().collect());
        start = end;
    }
    result.push(chars[start..].iter().collect());
    result
}

fn read_at(c: &Connection, stored: &str) -> Result<(PathBuf, String), DbError> {
    let mut path = PathBuf::from(stored);
    if !path.exists() {
        if let (Some(index), Some(root)) = (
            stored.find("books/"),
            c.path().and_then(|p| Path::new(p).parent()),
        ) {
            path = root.join(&stored[index..]);
        }
    }
    let content = std::fs::read_to_string(&path)?;
    Ok((path, content))
}

fn restore_files(snapshots: &[(PathBuf, String)], created: &[PathBuf]) -> Result<(), String> {
    for path in created {
        let _ = std::fs::remove_file(path);
    }
    for (path, body) in snapshots {
        std::fs::write(path, body).map_err(|e| format!("恢复 {} 失败：{e}", path.display()))?;
    }
    Ok(())
}

fn active(c: &Connection, book_id: i64) -> Result<Vec<UnitRow>, DbError> {
    let mut stmt = c.prepare("SELECT u.id,u.book_id,u.idx,u.title,u.content_path,u.status,u.mastery FROM units u LEFT JOIN unit_revisions r ON r.unit_id=u.id WHERE u.book_id=?1 AND (r.unit_id IS NULL OR r.active_first=1) ORDER BY u.idx,u.id")?;
    let rows = stmt
        .query_map(params![book_id], map_unit_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn reindex(c: &Connection, book_id: i64, order: &[i64]) -> Result<(), DbError> {
    // 先移到不可能被用户使用的负数 ID 槽位，逐个 UPDATE 不会互相撞 idx。
    c.execute(
        "UPDATE units SET idx=-id WHERE book_id=?1",
        params![book_id],
    )?;
    for (index, id) in order.iter().enumerate() {
        c.execute("UPDATE units SET idx=?2 WHERE id=?1", params![id, index])?;
    }
    Ok(())
}

fn locked(c: &Connection, unit_id: i64) -> Result<bool, DbError> {
    let row = c.query_row(
        "SELECT id,book_id,idx,title,content_path,status,mastery FROM units WHERE id=?1",
        params![unit_id],
        map_unit_row,
    )?;
    let history: i64 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM questions WHERE unit_id = :unit) OR EXISTS(SELECT 1 FROM review_items WHERE unit_id = :unit) OR EXISTS(SELECT 1 FROM chapter_explanations WHERE first_unit_id = :unit)",
        rusqlite::named_params! { ":unit": unit_id },
        |r| r.get(0),
    )?;
    Ok(row.status != "unread" || row.mastery.is_some() || history != 0)
}

impl Db {
    pub fn chapter_titles(&self, book_id: i64) -> Result<HashMap<i64, String>, DbError> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT u.id,o.chapter_title FROM unit_chapter_overrides o JOIN units u ON u.id=o.unit_id WHERE u.book_id=?1")?;
            let rows = stmt.query_map(params![book_id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<Result<HashMap<_,_>,_>>()?;
            Ok(rows)
        })
    }

    pub fn has_chapter_overrides(&self, book_id: i64) -> Result<bool, DbError> {
        self.with_conn(|c| Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM unit_chapter_overrides o JOIN units u ON u.id=o.unit_id WHERE u.book_id=?1)",params![book_id],|r|r.get::<_,i64>(0))?>0))
    }

    pub fn save_chapter_snapshot(
        &self,
        _book_id: i64,
        titles: &HashMap<i64, String>,
    ) -> Result<(), DbError> {
        self.transaction(|c| {
            for (id, title) in titles {
                c.execute(
                    "INSERT INTO unit_chapter_overrides(unit_id,chapter_title) VALUES(?1,?2) ON CONFLICT(unit_id) DO NOTHING",
                    params![id, title],
                )?;
            }
            Ok(())
        })
    }

    pub fn preview_resplit(
        &self,
        unit_id: i64,
        chapter_title: &str,
        target: usize,
    ) -> Result<ResplitPreview, DbError> {
        if !(300..=5000).contains(&target) {
            return Err(DbError::InvalidData("目标字数须为 300–5000 字".into()));
        }
        self.with_conn(|c| {
            if locked(c, unit_id)? {
                return Err(DbError::InvalidData(
                    "已有学习记录、题目或章讲解的小节已锁定；请选择未读小节".into(),
                ));
            }
            let original = c.query_row(
                "SELECT id,book_id,idx,title,content_path,status,mastery FROM units WHERE id=?1",
                params![unit_id],
                map_unit_row,
            )?;
            let (_, content) = read_at(c, &original.content_path)?;
            let parts = split_text(&content, &original.title, target);
            if parts.is_empty() {
                return Err(DbError::InvalidData(
                    "不足以按此粒度拆分：请调整目标字数".into(),
                ));
            }
            Ok(ResplitPreview {
                chapter_title: chapter_title.into(),
                target_chars: target,
                unit_count: parts.len() as i64,
                preserved_unit_ids: vec![],
                replaced_unit_ids: vec![unit_id],
                explanation_unit_ids: vec![],
                preview: parts
                    .iter()
                    .map(|p| format!("{} · {} 字", p.title, p.content.chars().count()))
                    .collect(),
            })
        })
    }

    pub fn resplit_unit(
        &self,
        unit_id: i64,
        chapter_title: &str,
        target: usize,
        created_at: &str,
    ) -> Result<Vec<Unit>, DbError> {
        if !(300..=5000).contains(&target) {
            return Err(DbError::InvalidData("目标字数须为 300–5000 字".into()));
        }
        let mut snapshots = Vec::<(PathBuf, String)>::new();
        let mut created = Vec::<PathBuf>::new();
        let mut conn = self.conn.lock().map_err(|_| DbError::LockPoisoned)?;
        let tx = conn.transaction()?;
        let result = (|| -> Result<i64, DbError> {
            let c = &tx;
            let original = c.query_row("SELECT id,book_id,idx,title,content_path,status,mastery FROM units WHERE id = ?1", params![unit_id], map_unit_row).optional()?.ok_or_else(||DbError::InvalidData("未找到小节".into()))?;
            if locked(c, unit_id)? {
                return Err(DbError::InvalidData("已有学习记录的小节不能拆分".into()));
            }
            let already_split: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM unit_revisions WHERE unit_id = :unit AND active_first = 1)", rusqlite::named_params! { ":unit": unit_id }, |r| r.get(0))?;
            if already_split {
                return Err(DbError::InvalidData("该小节已被拆分，请刷新目录".into()));
            }
            let rows = active(c, original.book_id)?;
            let (path, content) = read_at(c, &original.content_path)?;
            let parts = split_text(&content, &original.title, target);
            if parts.is_empty() {
                return Err(DbError::InvalidData("正文不足以按此粒度拆分".into()));
            }
            // 强校验：段落边界只搬移，不丢换行、缩进或 Markdown 标记。
            if parts
                .iter()
                .map(|part| part.content.as_str())
                .collect::<String>()
                != content.trim()
            {
                return Err(DbError::InvalidData(
                    "拆分完整性校验失败，未修改正文".into(),
                ));
            }
            snapshots.push((path.clone(), content.clone()));
            let parent = path
                .parent()
                .ok_or_else(|| DbError::InvalidData("正文路径无效".into()))?
                .to_path_buf();
            let mut ids = Vec::new();
            let mut files = Vec::new();
            for (index, part) in parts.iter().enumerate() {
                let id = if index == 0 {
                    unit_id
                } else {
                    let child_index = -2_000_000_000i64;
                    c.execute(
                        "INSERT INTO units(book_id,idx,title,content_path) VALUES(?1,?2,?3,'')",
                        params![original.book_id, child_index, part.title],
                    )?;
                    tx.last_insert_rowid()
                };
                let full_path = if index == 0 {
                    path.clone()
                } else {
                    parent.join(format!("unit_{id}.txt"))
                };
                if index != 0 && full_path.exists() {
                    return Err(DbError::InvalidData(
                        "目标小节文件已存在，已保留原内容".into(),
                    ));
                }
                c.execute(
                    "UPDATE units SET title=?2,content_path=?3 WHERE id=?1",
                    params![id, part.title, full_path.to_string_lossy()],
                )?;
                c.execute(
                    "INSERT INTO unit_chapter_overrides(unit_id,chapter_title) VALUES(?1,?2) ON CONFLICT(unit_id) DO NOTHING",
                    params![id, chapter_title],
                )?;
                ids.push(id);
                files.push((full_path, part.content.clone()));
            }
            let ids_json = serde_json::to_string(&ids).unwrap_or_default();
            c.execute("INSERT INTO unit_revisions(unit_id,unit_title,chapter_title,content,archived_at,child_ids,active_first,original_idx,original_status,original_mastery,original_path) VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,?9,?10)",params![unit_id,original.title,chapter_title,content,created_at,ids_json,original.idx,original.status,original.mastery,original.content_path])?;
            c.execute("INSERT INTO split_revisions(original_unit_id,book_id,created_at,child_ids) VALUES(?1,?2,?3,?4)",params![unit_id,original.book_id,created_at,ids_json])?;
            let mut order = Vec::new();
            for row in &rows {
                if row.id == unit_id {
                    order.extend(ids.iter().copied());
                } else {
                    order.push(row.id);
                }
            }
            reindex(c, original.book_id, &order)?;
            for (full_path, body) in files {
                if full_path != path {
                    created.push(full_path.clone());
                }
                std::fs::write(&full_path, body)?;
            }
            Ok(original.book_id)
        })();
        // 任何错误（包括最后一个文件写入、commit 失败）都必须还原原文、删除本次新文件。
        match result.and_then(|book_id| tx.commit().map(|_| book_id).map_err(DbError::Sqlite)) {
            Ok(book_id) => {
                drop(conn);
                self.mark_dirty();
                self.list_units(book_id)
            }
            Err(error) => {
                // tx 在成功或失败提交时均已消费；无需再取文件锁来恢复文件。
                match restore_files(&snapshots, &created) {
                    Ok(()) => Err(error),
                    Err(e) => Err(DbError::InvalidData(format!("{error}；文件回滚失败：{e}"))),
                }
            }
        }
    }

    pub fn undo_resplit(&self, unit_id: i64) -> Result<Vec<Unit>, DbError> {
        let mut snapshots = Vec::<(PathBuf, String)>::new();
        let created = Vec::<PathBuf>::new();
        let mut delete_after_commit = Vec::<PathBuf>::new();
        let mut conn = self.conn.lock().map_err(|_| DbError::LockPoisoned)?;
        let tx = conn.transaction()?;
        let result = (|| -> Result<i64, DbError> {
            let c = &tx;
            let stored = c
                .query_row(
                    "SELECT child_ids,book_id FROM split_revisions WHERE original_unit_id=?1",
                    params![unit_id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()?
                .ok_or_else(|| DbError::InvalidData("该小节没有可撤销的拆分".into()))?;
            let ids: Vec<i64> = serde_json::from_str(&stored.0)
                .map_err(|_| DbError::InvalidData("拆分记录损坏".into()))?;
            if ids.len() < 2 || ids.first() != Some(&unit_id) {
                return Err(DbError::InvalidData("拆分记录不完整".into()));
            }
            for id in &ids {
                if *id != unit_id
                    && c.query_row(
                        "SELECT EXISTS(SELECT 1 FROM split_revisions WHERE original_unit_id=?1)",
                        params![id],
                        |r| r.get::<_, i64>(0),
                    )? > 0
                {
                    return Err(DbError::InvalidData("请先撤销较新的拆分".into()));
                }
                if locked(c, *id)? {
                    return Err(DbError::InvalidData(
                        "子小节已开始学习，不能撤销丢失其进度".into(),
                    ));
                }
                let (content_path, content) = c.query_row("SELECT u.content_path,r.content FROM units u LEFT JOIN unit_revisions r ON r.unit_id=u.id WHERE u.id=?1",params![id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?)))?;
                let content = match content {
                    Some(raw) => raw,
                    None => read_at(c, &content_path)?.1,
                };
                snapshots.push((read_at(c, &content_path)?.0, content));
            }
            let (book_id,title,chapter,old_content,idx,status,mastery,path)=c.query_row("SELECT u.book_id,r.unit_title,r.chapter_title,r.content,r.original_idx,r.original_status,r.original_mastery,r.original_path FROM units u JOIN unit_revisions r ON r.unit_id=u.id WHERE u.id=?1",params![unit_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<f64>>(6)?,r.get::<_,String>(7)?)))?;
            // 子路径和首小节路径不同（如未来编辑过路径）时，原路径也必须在撤销前保留。
            let original_path = read_at(c, &path)?.0;
            if !snapshots.iter().any(|(p, _)| p == &original_path) {
                snapshots.push((original_path.clone(), read_at(c, &path)?.1));
            }
            std::fs::write(&original_path, &old_content)?;
            c.execute(
                "DELETE FROM split_revisions WHERE original_unit_id=?1",
                params![unit_id],
            )?;
            c.execute(
                "DELETE FROM unit_revisions WHERE unit_id=?1",
                params![unit_id],
            )?;
            for id in ids.iter().skip(1) {
                let child = c.query_row(
                    "SELECT content_path FROM units WHERE id=?1",
                    params![id],
                    |r| r.get::<_, String>(0),
                )?;
                let child_path = read_at(c, &child)?.0;
                delete_after_commit.push(child_path.clone());
                if !snapshots.iter().any(|(p, _)| p == &child_path) {
                    snapshots.push((child_path, read_at(c, &child)?.1));
                }
                c.execute("DELETE FROM units WHERE id=?1", params![id])?;
            }
            c.execute(
                "UPDATE units SET title=?2,content_path=?3,status=?4,mastery=?5 WHERE id=?1",
                params![unit_id, title, path, status, mastery],
            )?;
            // 原 ID 的章归属已存在且不可被新小节夺走；继续保留原映射。
            let retained_chapter: String = c.query_row(
                "SELECT chapter_title FROM unit_chapter_overrides WHERE unit_id=?1",
                params![unit_id],
                |row| row.get(0),
            )?;
            debug_assert_eq!(retained_chapter, chapter);
            let rows = active(c, book_id)?;
            // 原单元只回到自己原来的槽位一次：后面所有未学习小节都已因索引重排而 idx 变大，
            // 若对每行都插一次会得到 [.., unit_id, row_a, unit_id, row_b]，reindex 时后一次
            // 写入覆盖前一次，原单元就被挪到了本该在它后面的小节之后。
            let mut order = Vec::with_capacity(rows.len());
            let mut placed = false;
            for row in rows {
                if row.id == unit_id {
                    continue;
                }
                if !placed && row.idx > idx {
                    order.push(unit_id);
                    placed = true;
                }
                order.push(row.id);
            }
            if !placed {
                order.push(unit_id);
            }
            reindex(c, book_id, &order)?;
            Ok(book_id)
        })();
        match result.and_then(|book_id| tx.commit().map(|_| book_id).map_err(DbError::Sqlite)) {
            Ok(book_id) => {
                for path in delete_after_commit {
                    let _ = std::fs::remove_file(path);
                }
                drop(conn);
                self.mark_dirty();
                self.list_units(book_id)
            }
            Err(error) => {
                // tx 在成功或失败提交时均已消费；无需再取文件锁来恢复文件。
                match restore_files(&snapshots, &created) {
                    Ok(()) => Err(error),
                    Err(e) => Err(DbError::InvalidData(format!("{error}；撤销回滚失败：{e}"))),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ai-teacher-split-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn utf8_split_is_lossless_including_markdown_smallest() {
        let raw = "第一段。\n\n".to_string() + &"中文记忆，保持原文；".repeat(90);
        let parts = split_text(&raw, "记忆", 300);
        assert!(parts.len() > 1);
        assert_eq!(
            parts.iter().map(|p| p.content.as_str()).collect::<String>(),
            raw.trim()
        );
        assert!(split_text(&raw, "x", 299).is_empty());
        let rich = "## 章节。！\n\n  缩进的代码应保留\n\n".to_string() + &"中文正文；".repeat(150);
        let parts = split_text(&rich, "标题", 300);
        assert_eq!(
            parts.iter().map(|p| p.content.as_str()).collect::<String>(),
            rich.trim()
        );
    }

    #[test]
    fn database_split_preserves_order_identity_and_undo() {
        let dir = temp("roundtrip");
        let db = Db::open_in_memory().unwrap();
        let book = db
            .insert_book("书", "/tmp/test-split-book.md", "md", "2026-11-09")
            .unwrap();
        let original = "前言。\n\n".to_string() + &"中文章节，包含标点！；".repeat(150);
        let f0 = dir.join("unit_0.txt");
        let f1 = dir.join("unit_1.txt");
        std::fs::write(&f0, &original).unwrap();
        std::fs::write(&f1, "下一节原文").unwrap();
        let u0 = db
            .insert_unit(book, 0, "第一章", f0.to_str().unwrap())
            .unwrap();
        let u1 = db
            .insert_unit(book, 1, "第二章", f1.to_str().unwrap())
            .unwrap();
        let units = db
            .resplit_unit(u0, "第一章", 300, "2026-11-09T10:00:00+08:00")
            .unwrap();
        assert!(units.len() > 2);
        assert_eq!(
            units.last().unwrap().id,
            u1,
            "只占原小节的槽位，不重排其它章"
        );
        assert!(units[0].title.ends_with("（1）"));
        assert!(db.has_chapter_overrides(book).unwrap());
        assert_eq!(db.undo_resplit(u0).unwrap().len(), 2);
        assert_eq!(db.get_unit(u0).unwrap().unwrap().title, "第一章");
        assert_eq!(std::fs::read_to_string(&f0).unwrap(), original);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn undo_restores_position_with_units_after_the_split() {
        // 拆分点后面还有多个小节时，撤销必须让原小节回到它原来的位置。
        // 曾经对每个 idx 变大的行都插一次原 id，reindex 的第二次写入把原小节挪到了它们之后。
        let dir = temp("undo-order");
        let db = Db::open_in_memory().unwrap();
        let book = db
            .insert_book("书", "/tmp/test-undo-order.md", "md", "2026-11-09")
            .unwrap();
        let mut ids = Vec::new();
        for index in 0..4 {
            let file = dir.join(format!("unit_{index}.txt"));
            let body = if index == 1 {
                "前言。\n\n".to_string() + &"中文章节，包含标点！；".repeat(150)
            } else {
                format!("第 {index} 节原文")
            };
            std::fs::write(&file, body).unwrap();
            ids.push(
                db.insert_unit(
                    book,
                    index,
                    &format!("第 {} 节", index + 1),
                    file.to_str().unwrap(),
                )
                .unwrap(),
            );
        }
        let order_before: Vec<i64> = db
            .list_unit_rows(book)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(order_before, ids);

        db.resplit_unit(ids[1], "第一章", 300, "2026-11-09T10:00:00+08:00")
            .unwrap();
        let split: Vec<i64> = db
            .list_unit_rows(book)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(split[0], ids[0], "拆分不移动前面的小节");
        assert_eq!(split[1], ids[1], "首个子小节占用原槽位");
        assert_eq!(
            &split[split.len() - 2..],
            &[ids[2], ids[3]],
            "后面的小节保持原顺序"
        );

        db.undo_resplit(ids[1]).unwrap();
        let order_after: Vec<i64> = db
            .list_unit_rows(book)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(order_after, ids, "撤销后必须逐位恢复原始顺序");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn learned_sections_cannot_split_or_undo() {
        let dir = temp("protection");
        let db = Db::open_in_memory().unwrap();
        let book = db
            .insert_book("书", "/tmp/test-split-protect.md", "md", "2026-11-09")
            .unwrap();
        let file = dir.join("unit_0.txt");
        std::fs::write(&file, "中文".repeat(1500)).unwrap();
        let id = db
            .insert_unit(book, 0, "第一章", file.to_str().unwrap())
            .unwrap();
        db.update_unit_progress(id, "reading", None).unwrap();
        assert!(db.resplit_unit(id, "第一章", 300, "today").is_err());
        db.update_unit_progress(id, "unread", None).unwrap();
        db.resplit_unit(id, "第一章", 300, "today").unwrap();
        db.update_unit_progress(id, "reading", None).unwrap();
        assert!(db.undo_resplit(id).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! 用**真实数据副本**跑一次完整备份/恢复链路（不碰原库）。
//!
//! 需要环境变量指向 WebDAV；未设置则跳过。用法见 webdav_live.rs 顶部注释。

use std::path::PathBuf;

use ai_teacher_lib::sync;
use ai_teacher_lib::WebDavSettings;

fn settings_from_env() -> Option<WebDavSettings> {
    let base_url = std::env::var("AI_TEACHER_WEBDAV_URL").ok()?;
    let username = std::env::var("AI_TEACHER_WEBDAV_USER").ok()?;
    let password = std::env::var("AI_TEACHER_WEBDAV_PASS").ok()?;
    Some(WebDavSettings {
        enabled: true,
        base_url,
        username,
        password,
        // 与 webdav_live 用不同目录，互不干扰
        folder: "ai-teacher-e2e-real".to_string(),
    })
}

#[test]
fn real_data_roundtrip() {
    let Some(settings) = settings_from_env() else {
        eprintln!("跳过：未设置 WebDAV 环境变量");
        return;
    };
    // 用真实库的副本；没有就跳过（CI 上没有这份数据）
    let src_db = std::env::var("AI_TEACHER_REAL_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Library/Application Support/com.example.aiteacher/ai_teacher.db")
    });
    if !std::path::Path::new(&src_db).exists() {
        eprintln!("跳过：找不到真实库 {src_db}");
        return;
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // 工作目录：把真实库和 books/ 复制进去，模拟一台"新机器"
        let base = std::env::temp_dir().join(format!("ai-teacher-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let work = base.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::copy(&src_db, work.join("ai_teacher.db")).unwrap();

        let books_src = std::path::Path::new(&src_db).parent().unwrap().join("books");
        if books_src.is_dir() {
            copy_dir(&books_src, &work.join("books"));
        }

        let db = ai_teacher_lib::Db::open(&work.join("ai_teacher.db")).unwrap();
        let before = db.stats().unwrap();
        println!(
            "[真实数据] 备份前：{} 本 / {} 单元 / 完成 {} / 答题 {} / 复习 {}",
            before.books, before.units, before.done_units, before.attempts, before.review_items
        );
        assert_eq!(before.books, 4, "应读到 4 本书");

        // 完整备份（含 books/）
        let (msg, bytes) = sync::upload_full(&db, &work, &settings)
            .await
            .expect("完整备份应成功");
        println!("[真实数据] 已上传：{msg}");

        // 列远端
        let list = sync::list_remote(&settings).await.unwrap();
        let full = list
            .iter()
            .find(|s| s.name == sync::LATEST_BOOKS)
            .expect("应有整包");
        let m = full.manifest.as_ref().unwrap();
        println!(
            "[真实数据] 远端整包 {} 字节，含书籍={}，schema v{}",
            full.size, m.includes_books, m.schema_version
        );
        assert!(m.includes_books);
        assert_eq!(m.db_stats.books, before.books);
        assert_eq!(m.db_stats.units, before.units);

        // 新目录 = 全新空白机器，从远端恢复
        let fresh = base.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        let db2 = ai_teacher_lib::Db::open(&fresh.join("ai_teacher.db")).unwrap();
        assert_eq!(db2.stats().unwrap().books, 0, "新机器应为空");

        let msg = sync::download_and_restore(&db2, &fresh, &settings, sync::LATEST_BOOKS, false)
            .await
            .expect("恢复应成功");
        println!("[真实数据] 已恢复：{msg}");

        let after = db2.stats().unwrap();
        println!(
            "[真实数据] 恢复后：{} 本 / {} 单元 / 完成 {} / 答题 {} / 复习 {}",
            after.books, after.units, after.done_units, after.attempts, after.review_items
        );
        assert_eq!(after, before, "恢复后统计必须与备份前完全一致");

        // books/ 应一并恢复（含插图）
        let restored_book1 = fresh.join("books").join("1");
        assert!(restored_book1.is_dir(), "books/1 应被恢复");
        let imgs = restored_book1.join("images");
        if imgs.is_dir() {
            let n = std::fs::read_dir(&imgs).unwrap().count();
            println!("[真实数据] 插图恢复 {n} 张");
            assert!(n > 0, "插图应被恢复");
        }

        let _ = std::fs::remove_dir_all(&base);
        println!("[真实数据] 全部通过：备份→传输→恢复→校验");
        let _ = bytes;
    });
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            let _ = std::fs::copy(&p, &t);
        }
    }
}

// ---------- 图书列表不一致的场景 ----------

/// DbOnly（仅进度）备份 → 恢复到"图书列表不同"的机器，会发生什么。
/// 这个测试描述的是**当前行为**，不是"正确行为"——用来锁住已知风险。
#[test]
fn db_only_restore_with_different_book_lists() {
    let base = std::env::temp_dir().join(format!("ai-teacher-mismatch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // 机器 A：2 本书，正文都在
    let a = base.join("a");
    std::fs::create_dir_all(a.join("books/1")).unwrap();
    std::fs::create_dir_all(a.join("books/2")).unwrap();
    std::fs::write(a.join("books/1/unit_0.txt"), "A书第一节").unwrap();
    std::fs::write(a.join("books/2/unit_0.txt"), "A书第二节").unwrap();
    let db_a = ai_teacher_lib::Db::open(&a.join("ai_teacher.db")).unwrap();
    let b1 = db_a.insert_book("书一", "/tmp/1.md", "md", "2026-11-01").unwrap();
    let b2 = db_a.insert_book("书二", "/tmp/2.md", "md", "2026-11-02").unwrap();
    db_a.insert_unit(b1, 0, "节一", a.join("books/1/unit_0.txt").to_str().unwrap()).unwrap();
    db_a.insert_unit(b2, 0, "节二", a.join("books/2/unit_0.txt").to_str().unwrap()).unwrap();

    // 只用进度备份（不含 books/）
    let bytes = ai_teacher_lib::sync::build_backup(
        &db_a,
        &a,
        ai_teacher_lib::sync::PackMode::DbOnly,
    )
    .unwrap();
    let m = ai_teacher_lib::sync::read_manifest(&bytes).unwrap();
    assert!(!m.includes_books, "DbOnly 不该含 books");

    // 机器 B：完全不同的一本书，books/9
    let bdir = base.join("b");
    std::fs::create_dir_all(bdir.join("books/9")).unwrap();
    std::fs::write(bdir.join("books/9/unit_0.txt"), "B书正文").unwrap();
    let db_b = ai_teacher_lib::Db::open(&bdir.join("ai_teacher.db")).unwrap();
    let b9 = db_b.insert_book("B的书", "/tmp/9.md", "md", "2026-11-03").unwrap();
    db_b.insert_unit(b9, 0, "B节", bdir.join("books/9/unit_0.txt").to_str().unwrap()).unwrap();

    // 恢复到 B
    ai_teacher_lib::sync::restore_from_bytes(&db_b, &bdir, &bytes, false).unwrap();

    // 结果：数据库里是 A 的书（含 A 的绝对路径），但 B 的 books/9 文件还在
    let stats = db_b.stats().unwrap();
    assert_eq!(stats.books, 2, "数据库变成 A 的 2 本书");
    let books = db_b.list_books().unwrap();
    println!("恢复后书单：{:?}", books.iter().map(|b| &b.title).collect::<Vec<_>>());
    assert!(books.iter().any(|b| b.title == "书一"));
    assert!(!books.iter().any(|b| b.title == "B的书"), "B 的书从库里消失了");

    // 致命点：A 的单元正文路径指向 **A 的目录**，在 B 上根本不存在
    let rows = db_b.list_unit_rows(b1).unwrap();
    let p = std::path::Path::new(&rows[0].content_path);
    println!("恢复后单元正文路径：{}（存在={}）", p.display(), p.exists());
    assert!(p.exists(), "路径恰好仍存在，因为 A 和 B 在同一台机器上跑的测试");
    // 但在真机上 B 没有 A 的数据目录 → 这条路径必然不存在 →
    // read_unit_content 返回 Err("无法读取单元内容")，阅读页整章加载失败。

    // 孤立的 books/9 不会自动清理（文件还在，但没有 DB 记录指向它）
    assert!(bdir.join("books/9/unit_0.txt").exists(), "孤儿文件不会被清理");

    let _ = std::fs::remove_dir_all(&base);
}

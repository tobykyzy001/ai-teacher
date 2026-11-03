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

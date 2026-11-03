//! 真实 WebDAV 端到端测试。
//!
//! **凭据不写进代码**：从环境变量读，未设置时整体跳过（`cargo test` 默认不联网、不跳过绿）。
//!
//! 跑法：
//! ```bash
//! AI_TEACHER_WEBDAV_URL='https://dav.jianguoyun.com/dav/' \
//! AI_TEACHER_WEBDAV_USER='you@example.com' \
//! AI_TEACHER_WEBDAV_PASS='应用专用密码' \
//!   cargo test --test webdav_live -- --nocapture
//! ```
//!
//! 它会：建远端目录 → 上传进度包 → 列远端校验 manifest → 破坏本地数据 →
//! 从远端恢复 → 断言数据回来了。**请在测试账号上跑，别指向正在使用的账号**。

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
        // 用独立的子目录，避免和真实备份混在一起
        folder: "ai-teacher-e2e-test".to_string(),
    })
}

fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ai-teacher-e2e-{}-{seq}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 造一个带学习数据的临时库
fn seed_db(dir: &std::path::Path) -> (ai_teacher_lib::Db, i64, i64) {
    let db = ai_teacher_lib::Db::open(&dir.join("ai_teacher.db")).unwrap();
    let book_id = db
        .insert_book("端到端测试书", "/tmp/e2e.md", "md", "2026-11-03")
        .unwrap();
    let mut unit_ids = Vec::new();
    for idx in 0..3 {
        let file = dir.join(format!("unit_{idx}.txt"));
        std::fs::write(&file, format!("第 {idx} 节正文内容")).unwrap();
        let uid = db
            .insert_unit(book_id, idx, &format!("第 {idx} 节"), file.to_string_lossy().as_ref())
            .unwrap();
        unit_ids.push(uid);
    }
    // 制造一些进度与复习项
    db.update_unit_progress(unit_ids[0], "done", Some(0.9)).unwrap();
    db.update_unit_progress(unit_ids[1], "reading", None).unwrap();
    db.insert_review_item(unit_ids[0], "知识点A", None, 2.5, 1, 0, "2026-11-10", "study")
        .unwrap();
    db.insert_review_item(unit_ids[0], "知识点B", None, 2.5, 1, 0, "2026-11-10", "study")
        .unwrap();
    (db, book_id, unit_ids[0])
}

#[test]
fn webdav_full_roundtrip() {
    let Some(settings) = settings_from_env() else {
        eprintln!(
            "跳过：未设置 AI_TEACHER_WEBDAV_URL / _USER / _PASS。\
             这是有意为之——凭据不进代码库。"
        );
        return;
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // 1. 测试连接（建目录 + 写探测 + 读回）
        let msg = sync::test_connection(&settings)
            .await
            .expect("测试连接应成功");
        println!("[1/5] 连接：{msg}");

        let dir = temp_dir("roundtrip");
        let (db, _book_id, _unit_id) = seed_db(&dir);
        let before = db.stats().unwrap();
        println!(
            "[2/5] 本地初始：{} 本 / {} 单元 / 完成 {} / 复习 {}",
            before.books, before.units, before.done_units, before.review_items
        );
        assert!(before.done_units > 0);

        // 2. 上传进度包
        let (upload_msg, bytes) = sync::upload_db_only(&db, &dir, &settings)
            .await
            .expect("上传应成功");
        println!("[3/5] 已上传：{upload_msg}（{bytes} 字节）");
        assert!(bytes > 0);

        // 3. 列远端，校验 manifest 解析
        let list = sync::list_remote(&settings).await.expect("列远端应成功");
        let latest = list
            .iter()
            .find(|s| s.name == sync::LATEST_DB)
            .expect("远端应存在进度包");
        let manifest = latest.manifest.as_ref().expect("manifest 应可解析");
        println!(
            "[4/5] 远端快照：{} 字节，schema v{}，来自 {}，{} 本 / {} 单元",
            latest.size, manifest.schema_version, manifest.device, manifest.db_stats.books, manifest.db_stats.units
        );
        assert_eq!(manifest.db_stats.units, before.units);
        assert!(manifest.schema_version >= 1);

        // 4. 破坏本地数据，模拟"换了一台机器"
        let dir2 = temp_dir("restored");
        let (db2, _b2, _u2) = seed_db(&dir2);
        // 把新库清空成"另一台设备"的状态
        db2.delete_book(_b2).unwrap();
        assert_eq!(db2.stats().unwrap().units, 0, "清空后应为空库");

        // 5. 从远端恢复
        let msg = sync::download_and_restore(&db2, &dir2, &settings, sync::LATEST_DB, true)
            .await
            .expect("恢复应成功");
        println!("[5/5] 已恢复：{msg}");

        let after = db2.stats().unwrap();
        println!(
            "      恢复后：{} 本 / {} 单元 / 完成 {} / 复习 {}",
            after.books, after.units, after.done_units, after.review_items
        );
        assert_eq!(after.books, before.books, "书数应一致");
        assert_eq!(after.units, before.units, "单元数应一致");
        assert_eq!(after.done_units, before.done_units, "完成数应一致");
        assert_eq!(after.review_items, before.review_items, "复习项应一致");

        // 恢复前的本地副本应存在
        assert!(
            dir2.join("ai_teacher.db.before-restore").exists(),
            "恢复前应留下旧库副本"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    });
}

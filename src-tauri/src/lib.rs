mod commands;
mod config;
mod db;
mod ingest;
mod llm;
mod models;
mod scheduler;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("无法获取应用数据目录：{e}"))?;
            std::fs::create_dir_all(&data_dir).map_err(|e| format!("无法创建应用数据目录：{e}"))?;
            std::fs::create_dir_all(data_dir.join("books"))
                .map_err(|e| format!("无法创建书籍目录：{e}"))?;
            let database = db::Db::open(&data_dir.join("ai_teacher.db"))?;
            app.manage(commands::AppState {
                db: database,
                app_data_dir: data_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::test_llm_connection,
            commands::import_book,
            commands::list_books,
            commands::remove_book,
            commands::get_book_detail,
            commands::get_today,
            commands::get_unit_content,
            commands::start_reading,
            commands::ask,
            commands::ask_selection,
            commands::generate_quiz,
            commands::get_question,
            commands::list_wrong_questions,
            commands::submit_answer,
            commands::start_review,
            commands::submit_review,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

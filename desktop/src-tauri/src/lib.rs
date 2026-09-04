mod ai;
mod knowledge;
mod updater;
mod yemot;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            knowledge::get_knowledge_files,
            knowledge::get_knowledge_file_content,
            knowledge::search_knowledge_files,
            updater::check_for_updates,
            yemot::check_yemot_token,
            yemot::request_yemot_mfa,
            yemot::verify_yemot_mfa,
            yemot::execute_yemot_action,
            yemot::login_yemot,
            yemot::get_mfa_methods,
            yemot::send_mfa_code,
            yemot::validate_mfa_code,
            yemot::logout_yemot,
            ai::send_ai_request,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

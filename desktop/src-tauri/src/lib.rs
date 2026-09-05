mod agent;
mod ai;
mod knowledge;
mod secrets;
mod updater;
mod yemot;
mod yemot_ini;
mod yemot_inspect;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Native file picker for audio attachments. The UI only learns local
        // paths from it; Rust validates them before anything is uploaded.
        .plugin(tauri_plugin_dialog::init())
        .manage(agent::AgentRegistry::default())
        .invoke_handler(tauri::generate_handler![
            knowledge::get_knowledge_files,
            knowledge::get_knowledge_file_content,
            knowledge::search_knowledge_files,
            updater::check_for_updates,
            yemot::check_yemot_token,
            // `execute_yemot_actions` is still the script-mode write path in
            // +page.svelte; the single-key / MFA-modal / ext-preview commands it
            // used to sit next to had no caller left and are gone.
            yemot::execute_yemot_actions,
            yemot::login_yemot,
            yemot::get_mfa_methods,
            yemot::send_mfa_code,
            yemot::validate_mfa_code,
            yemot::logout_yemot,
            // Read-only line inspection for the editor UI.
            yemot_inspect::get_extension_tree,
            yemot_inspect::read_extension,
            ai::send_ai_request,
            agent::runner::start_agent_run,
            agent::runner::continue_agent_run,
            agent::runner::cancel_agent_run,
            agent::runner::approve_actions,
            agent::runner::undo_action,
            agent::runner::list_applied_changes,
            // Local task history ("היסטוריית משימות").
            agent::history::list_task_history,
            agent::history::get_task_history,
            agent::history::delete_task_history,
            agent::history::clear_task_history,
            secrets::secret_set, secrets::secret_get, secrets::secret_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

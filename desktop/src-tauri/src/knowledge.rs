use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};

// הטמעת כל תיקיית התיעוד והידע בתוך קובץ ה-Binary בזמן הקומפילציה
static KNOWLEDGE_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/knowledge");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct KnowledgeFileItem {
    pub name: String,
    pub size: usize,
}

#[tauri::command]
pub fn get_knowledge_files() -> Vec<KnowledgeFileItem> {
    let mut files = Vec::new();
    for file in KNOWLEDGE_DIR.files() {
        if let Some(name) = file.path().to_str() {
            files.push(KnowledgeFileItem {
                name: name.to_string(),
                size: file.contents().len(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

#[tauri::command]
pub fn get_knowledge_file_content(file_name: &str) -> Result<String, String> {
    if let Some(file) = KNOWLEDGE_DIR.get_file(file_name) {
        String::from_utf8(file.contents().to_vec())
            .map_err(|e| format!("שגיאת פענוח תוכן: {}", e))
    } else {
        Err(format!("קובץ הידע '{}' לא נמצא", file_name))
    }
}

#[tauri::command]
pub fn search_knowledge_files(query: &str) -> Vec<KnowledgeFileItem> {
    let q = query.trim().to_lowercase();
    let all = get_knowledge_files();
    if q.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|f| f.name.to_lowercase().contains(&q))
        .collect()
}

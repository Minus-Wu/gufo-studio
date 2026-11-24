use crate::gufo_manager::{GufoManager, GufoServerConfig, InferenceMetrics, ServerStatus};
use crate::rag::{
    EmbeddingTestResult, KnowledgeBaseDetail, KnowledgeBaseSummary, RagChunkPreview, RagConfig,
    RagImportResult, RagReindexResult, RagSearchResponse,
};
use crate::web_search::{WebSearchConfig, WebSearchResponse};
use crate::AppState;
use tauri::{AppHandle, State};

/// 出廠預設配置 (用於「還原預設配置」)。
#[tauri::command]
pub fn get_default_config() -> GufoServerConfig {
    GufoServerConfig::default()
}

/// 目前生效的配置 (以 gufo-studio.config.json 為唯一來源)。
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<GufoServerConfig, String> {
    Ok(state.manager.get_config().await)
}

#[tauri::command]
pub fn has_saved_config() -> bool {
    GufoManager::has_saved_config()
}

#[tauri::command]
pub async fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: GufoServerConfig,
) -> Result<(), String> {
    state.manager.save_config(Some(&app), config).await
}

#[tauri::command]
pub async fn get_server_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ServerStatus, String> {
    Ok(state.manager.get_status(Some(&app)).await)
}

#[tauri::command]
pub async fn start_server(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Option<GufoServerConfig>,
) -> Result<ServerStatus, String> {
    state.manager.start(app, config).await
}

#[tauri::command]
pub async fn stop_server(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ServerStatus, String> {
    state.manager.stop(Some(&app)).await
}

#[tauri::command]
pub async fn check_health(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.manager.check_health().await)
}

#[tauri::command]
pub async fn regenerate_https_cert(
    state: State<'_, AppState>,
    host: Option<String>,
    https_sans: Option<String>,
) -> Result<String, String> {
    state.manager.regenerate_https_cert(host, https_sans).await
}

#[tauri::command]
pub async fn get_recent_logs(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.manager.get_logs().await)
}

#[tauri::command]
pub async fn get_recent_metrics(state: State<'_, AppState>) -> Result<Vec<InferenceMetrics>, String> {
    Ok(state.manager.get_metrics().await)
}

#[tauri::command]
pub fn check_path_exists(path: String) -> bool {
    let p = std::path::Path::new(&path);
    if p.exists() {
        return true;
    }
    if p.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join(p).exists() {
                return true;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                if parent.join(p).exists() {
                    return true;
                }
            }
        }
    }
    false
}

#[tauri::command]
pub async fn select_file(
    title: Option<String>,
    filter_name: Option<String>,
    extensions: Option<Vec<String>>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();
        if let Some(t) = title {
            dialog = dialog.set_title(&t);
        }
        if let (Some(name), Some(exts)) = (filter_name, extensions) {
            let str_slices: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
            dialog = dialog.add_filter(&name, &str_slices);
        }
        if let Some(def) = default_path {
            let p = std::path::Path::new(&def);
            if p.exists() {
                if p.is_dir() {
                    dialog = dialog.set_directory(p);
                } else if let Some(parent) = p.parent() {
                    dialog = dialog.set_directory(parent);
                }
            } else if let Ok(cwd) = std::env::current_dir() {
                let full = cwd.join(p);
                if let Some(parent) = full.parent() {
                    if parent.exists() {
                        dialog = dialog.set_directory(parent);
                    }
                }
            }
        } else if let Ok(cwd) = std::env::current_dir() {
            dialog = dialog.set_directory(&cwd);
        }

        let picked = dialog.pick_file();
        if let Some(path_buf) = picked {
            // 若選取檔案位於工作目錄下，優先轉為簡潔的相對路徑
            if let Ok(cwd) = std::env::current_dir() {
                if let Ok(rel) = path_buf.strip_prefix(&cwd) {
                    return Ok(Some(rel.to_string_lossy().to_string()));
                }
            }
            return Ok(Some(path_buf.to_string_lossy().to_string()));
        }
        Ok(None)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── RAG 知識庫與檢索指令 ──

#[tauri::command]
pub async fn rag_get_config(state: State<'_, AppState>) -> Result<RagConfig, String> {
    Ok(state.rag.get_config().await)
}

#[tauri::command]
pub async fn rag_save_config(
    state: State<'_, AppState>,
    config: RagConfig,
) -> Result<RagConfig, String> {
    state.rag.save_config(config).await
}

#[tauri::command]
pub async fn rag_test_embedding(
    state: State<'_, AppState>,
    config: RagConfig,
) -> Result<EmbeddingTestResult, String> {
    state.rag.test_embedding(config).await
}

#[tauri::command]
pub async fn rag_list_kbs(
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeBaseSummary>, String> {
    Ok(state.rag.list_kbs().await)
}

#[tauri::command]
pub async fn rag_get_kb(
    state: State<'_, AppState>,
    kb_id: String,
) -> Result<KnowledgeBaseDetail, String> {
    state.rag.get_kb(&kb_id).await
}

#[tauri::command]
pub async fn rag_create_kb(
    state: State<'_, AppState>,
    name: String,
    description: String,
) -> Result<KnowledgeBaseSummary, String> {
    state.rag.create_kb(name, description).await
}

#[tauri::command]
pub async fn rag_delete_kb(state: State<'_, AppState>, kb_id: String) -> Result<(), String> {
    state.rag.delete_kb(&kb_id).await
}

#[tauri::command]
pub async fn rag_pick_files() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(|| {
        let mut dialog = rfd::FileDialog::new()
            .set_title("選取要匯入知識庫的文字或程式碼檔案")
            .add_filter(
                "純文字與程式碼文件",
                &[
                    "txt", "md", "markdown", "json", "jsonl", "csv", "tsv", "yaml", "yml", "toml",
                    "xml", "html", "css", "js", "jsx", "ts", "tsx", "rs", "py", "go", "java",
                    "c", "cpp", "h", "hpp", "cs", "sh", "ps1", "sql", "log",
                ],
            )
            .add_filter("所有檔案", &["*"]);
        if let Ok(cwd) = std::env::current_dir() {
            dialog = dialog.set_directory(&cwd);
        }
        let picked = dialog.pick_files().unwrap_or_default();
        Ok(picked
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn rag_pick_folder() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        let mut dialog = rfd::FileDialog::new().set_title("選取要批次匯入知識庫的資料夾");
        if let Ok(cwd) = std::env::current_dir() {
            dialog = dialog.set_directory(&cwd);
        }
        Ok(dialog
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn rag_add_files(
    state: State<'_, AppState>,
    kb_id: String,
    file_paths: Vec<String>,
) -> Result<RagImportResult, String> {
    state.rag.add_files(&kb_id, file_paths).await
}

#[tauri::command]
pub async fn rag_add_folder(
    state: State<'_, AppState>,
    kb_id: String,
    folder_path: String,
) -> Result<RagImportResult, String> {
    state.rag.add_folder(&kb_id, folder_path).await
}

#[tauri::command]
pub async fn rag_add_text(
    state: State<'_, AppState>,
    kb_id: String,
    title: String,
    content: String,
) -> Result<RagImportResult, String> {
    state.rag.add_text(&kb_id, title, content).await
}

#[tauri::command]
pub async fn rag_get_doc_chunks(
    state: State<'_, AppState>,
    kb_id: String,
    doc_id: String,
) -> Result<Vec<RagChunkPreview>, String> {
    state.rag.get_doc_chunks(&kb_id, &doc_id).await
}

#[tauri::command]
pub async fn rag_delete_document(
    state: State<'_, AppState>,
    kb_id: String,
    doc_id: String,
) -> Result<(), String> {
    state.rag.delete_document(&kb_id, &doc_id).await
}

#[tauri::command]
pub async fn rag_reindex_kb(
    state: State<'_, AppState>,
    kb_id: String,
) -> Result<RagReindexResult, String> {
    state.rag.reindex_kb(&kb_id).await
}

#[tauri::command]
pub async fn rag_search(
    state: State<'_, AppState>,
    kb_ids: Vec<String>,
    query: String,
    top_k: Option<u32>,
) -> Result<RagSearchResponse, String> {
    state.rag.search(kb_ids, query, top_k).await
}

// ── WebSearch 網路搜尋指令 ──

#[tauri::command]
pub async fn web_search_get_config(state: State<'_, AppState>) -> Result<WebSearchConfig, String> {
    Ok(state.web_search.get_config().await)
}

#[tauri::command]
pub async fn web_search_save_config(
    state: State<'_, AppState>,
    config: WebSearchConfig,
) -> Result<WebSearchConfig, String> {
    state.web_search.save_config(config).await
}

#[tauri::command]
pub async fn web_search_query(
    state: State<'_, AppState>,
    query: String,
    config_override: Option<WebSearchConfig>,
) -> Result<WebSearchResponse, String> {
    state.web_search.search(query, config_override).await
}

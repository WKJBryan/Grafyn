use crate::commands::sync_chunk_index_for_notes;
use crate::models::import::{ImportPreview, ImportResult};
use crate::models::note::{NoteCreate, NoteStatus};
use crate::services::import;
use crate::AppState;
use tauri::State;

/// Preview conversations in an import file (auto-detects format).
#[tauri::command]
pub async fn preview_import(
    file_path: String,
    _state: State<'_, AppState>,
) -> Result<ImportPreview, String> {
    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let platform = import::detect_platform(&content)
        .ok_or_else(|| "Could not detect conversation format".to_string())?;

    let conversations = import::parse_content(&content).map_err(|e| e.to_string())?;

    let total = conversations.len();

    Ok(ImportPreview {
        conversations,
        platform: platform.to_string(),
        total_conversations: total,
    })
}

/// Import selected conversations as container notes (evidence status).
#[tauri::command]
pub async fn apply_import(
    file_path: String,
    conversation_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let all_conversations = import::parse_content(&content).map_err(|e| e.to_string())?;

    // Filter to selected conversations (empty = import all)
    let to_import: Vec<_> = if conversation_ids.is_empty() {
        all_conversations
    } else {
        all_conversations
            .into_iter()
            .filter(|c| conversation_ids.contains(&c.id))
            .collect()
    };

    let mut note_ids = Vec::new();
    let mut created_notes = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = 0;

    for conv in &to_import {
        // Format conversation as markdown
        let markdown = import::format_as_markdown(conv);

        // Build tags: platform + import + suggested tags
        let mut tags = conv.suggested_tags.clone();
        if !tags.contains(&"import".to_string()) {
            tags.push("import".to_string());
        }
        tags.truncate(5);

        let note_create = NoteCreate {
            title: conv.title.clone(),
            content: markdown,
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags,
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: Some("import".to_string()),
            optimizer_managed: false,
            properties: {
                let mut props = std::collections::HashMap::new();
                props.insert(
                    "source".into(),
                    serde_json::Value::String(conv.platform.clone()),
                );
                props.insert(
                    "source_id".into(),
                    serde_json::Value::String(conv.id.clone()),
                );
                props.insert(
                    "created_via".into(),
                    serde_json::Value::String("import".into()),
                );
                if let Some(created_at) = conv.metadata.created_at {
                    props.insert(
                        "original_created_at".into(),
                        serde_json::Value::String(created_at.to_rfc3339()),
                    );
                }
                props
            },
        };

        // Create the note
        let created = {
            let mut store = state.knowledge_store.write().await;
            match store.create_note(note_create) {
                Ok(note) => note,
                Err(e) => {
                    errors.push(format!("Failed to create '{}': {}", conv.title, e));
                    skipped += 1;
                    continue;
                }
            }
        };

        // Index in search
        {
            let mut search = state.search_service.write().await;
            if let Err(e) = search.index_note(&created) {
                log::error!("Failed to index imported note '{}': {}", created.id, e);
            }
            if let Err(e) = search.commit() {
                log::error!("Failed to commit after indexing '{}': {}", created.id, e);
            }
        }

        // Update graph
        {
            let mut graph = state.graph_index.write().await;
            graph.update_note(&created);
        }

        note_ids.push(created.id.clone());
        created_notes.push(created);
    }

    sync_chunk_index_for_notes(state.inner(), &created_notes).await;

    let imported = note_ids.len();
    let message = format!(
        "Imported {} conversation{} as evidence notes",
        imported,
        if imported == 1 { "" } else { "s" }
    );

    Ok(ImportResult {
        imported,
        skipped,
        note_ids,
        errors,
        message,
    })
}

/// Get list of supported import formats.
#[tauri::command]
pub async fn get_supported_formats() -> Vec<String> {
    vec![
        "chatgpt".to_string(),
        "claude".to_string(),
        "grok".to_string(),
        "gemini".to_string(),
    ]
}

use super::shared::append_canvas_trace;
use crate::commands::commit_note_write;
use crate::models::canvas::{
    AvailableModel, CanvasSession, CanvasViewport, LLMNodePositionUpdate, ResponseStatus,
    SessionCreate, SessionMeta, SessionUpdate, TilePosition, TilePositionUpdate,
};
use crate::models::note::{NoteCreate, NoteStatus};
use crate::models::twin::TraceEventType;
use crate::AppState;
use serde_json::json;
use std::collections::HashMap;
use tauri::State;

/// List all canvas sessions
#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, String> {
    let mut store = state.canvas_store.write().await;
    store.list_sessions().map_err(|e| e.to_string())
}

/// Get a single session by ID
#[tauri::command]
pub async fn get_session(id: String, state: State<'_, AppState>) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    store.get_session(&id).map_err(|e| e.to_string())
}

/// Create a new canvas session
#[tauri::command]
pub async fn create_session(
    session: SessionCreate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    let created = store.create_session(session).map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &created.id,
        TraceEventType::SessionCreated,
        json!({
            "title": created.title.clone(),
            "description": created.description.clone(),
            "tags": created.tags.clone(),
            "status": created.status.clone(),
        }),
    )
    .await;

    Ok(created)
}

/// Update a canvas session
#[tauri::command]
pub async fn update_session(
    id: String,
    update: SessionUpdate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    let updated = store
        .update_session(&id, update)
        .map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &updated.id,
        TraceEventType::SessionUpdated,
        json!({
            "title": updated.title.clone(),
            "description": updated.description.clone(),
            "tags": updated.tags.clone(),
            "status": updated.status.clone(),
            "viewport": updated.viewport.clone(),
            "pinned_note_ids": updated.pinned_note_ids.clone(),
        }),
    )
    .await;

    Ok(updated)
}

/// Delete a canvas session
#[tauri::command]
pub async fn delete_session(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store.delete_session(&id).map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &id,
        TraceEventType::SessionDeleted,
        json!({
            "session_id": id,
        }),
    )
    .await;

    Ok(())
}

/// Get list of available LLM models
#[tauri::command]
pub async fn get_available_models(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, String> {
    let openrouter = state.openrouter.read().await;
    openrouter
        .get_available_models()
        .await
        .map_err(|e| e.to_string())
}

/// Update a tile's position
#[tauri::command]
pub async fn update_tile_position(
    session_id: String,
    tile_id: String,
    position: TilePositionUpdate,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .update_tile_position(&session_id, &tile_id, position)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a prompt tile and its children
#[tauri::command]
pub async fn delete_tile(
    session_id: String,
    tile_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .delete_tile(&session_id, &tile_id)
        .map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::TileDeleted,
        json!({
            "tile_id": tile_id,
        }),
    )
    .await;

    Ok(())
}

/// Delete a single model response from a tile
#[tauri::command]
pub async fn delete_response(
    session_id: String,
    tile_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .delete_response(&session_id, &tile_id, &model_id)
        .map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::ResponseDeleted,
        json!({
            "tile_id": tile_id,
            "model_id": model_id,
        }),
    )
    .await;

    Ok(())
}

/// Update viewport zoom/pan state
#[tauri::command]
pub async fn update_viewport(
    session_id: String,
    viewport: CanvasViewport,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .update_viewport(&session_id, viewport)
        .map_err(|e| e.to_string())
}

/// Update an LLM response node's position
#[tauri::command]
pub async fn update_llm_node_position(
    session_id: String,
    tile_id: String,
    model_id: String,
    position: LLMNodePositionUpdate,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .update_llm_node_position(&session_id, &tile_id, &model_id, position)
        .map_err(|e| e.to_string())
}

/// Auto-arrange all nodes (batch position update)
#[tauri::command]
pub async fn auto_arrange(
    session_id: String,
    positions: HashMap<String, TilePosition>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .batch_update_positions(&session_id, positions)
        .map_err(|e| e.to_string())
}

/// Export canvas session to a note (returns note info)
#[tauri::command]
pub async fn export_to_note(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    // Build markdown content from session
    let mut content = format!("# {}\n\n", session.title);

    if let Some(desc) = &session.description {
        content.push_str(&format!("{}\n\n", desc));
    }

    for tile in &session.prompt_tiles {
        content.push_str(&format!("## Prompt\n\n{}\n\n", tile.prompt));

        for (model_id, response) in &tile.responses {
            if response.status == ResponseStatus::Completed {
                content.push_str(&format!(
                    "### {} ({})\n\n{}\n\n",
                    response.model_name, model_id, response.content
                ));
            }
        }
    }

    for debate in &session.debates {
        content.push_str("## Debate\n\n");
        for round in &debate.rounds {
            content.push_str(&format!(
                "### Round {} - {}\n\n",
                round.round_number, round.topic
            ));
            for resp in &round.responses {
                content.push_str(&format!(
                    "**{} ({}):**\n\n{}\n\n",
                    resp.model_name, resp.model_id, resp.content
                ));
            }
        }
    }

    // Create note via knowledge store
    let mut ks = state.knowledge_store.write().await;

    let note = ks
        .create_note(NoteCreate {
            title: session.title.clone(),
            content,
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags: session.tags.clone(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        })
        .map_err(|e| e.to_string())?;

    drop(ks);

    commit_note_write(state.inner(), &note.id, "note_exported").await?;

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::NoteExported,
        json!({
            "note_id": note.id.clone(),
            "title": note.title.clone(),
        }),
    )
    .await;

    Ok(serde_json::json!({
        "note_id": note.id,
        "title": note.title,
        "updated": false,
    }))
}

use crate::models::canvas::{
    AvailableModel, CanvasSession, ModelResponse, PromptRequest, PromptTile, ResponseStatus,
    SessionCreate, SessionMeta, SessionUpdate, TilePositionUpdate,
};
use crate::services::openrouter::ChatMessage;
use crate::AppState;
use chrono::Utc;
use std::collections::HashMap;
use tauri::State;

/// List all canvas sessions
#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, String> {
    let store = state.canvas_store.read().await;
    store.list_sessions().map_err(|e| e.to_string())
}

/// Get a single session by ID
#[tauri::command]
pub async fn get_session(id: String, state: State<'_, AppState>) -> Result<CanvasSession, String> {
    let store = state.canvas_store.read().await;
    store.get_session(&id).map_err(|e| e.to_string())
}

/// Create a new canvas session
#[tauri::command]
pub async fn create_session(
    session: SessionCreate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    store.create_session(session).map_err(|e| e.to_string())
}

/// Update a canvas session
#[tauri::command]
pub async fn update_session(
    id: String,
    update: SessionUpdate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    store.update_session(&id, update).map_err(|e| e.to_string())
}

/// Delete a canvas session
#[tauri::command]
pub async fn delete_session(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store.delete_session(&id).map_err(|e| e.to_string())
}

/// Get list of available LLM models
#[tauri::command]
pub async fn get_available_models(state: State<'_, AppState>) -> Result<Vec<AvailableModel>, String> {
    let openrouter = state.openrouter.read().await;
    openrouter
        .get_available_models()
        .await
        .map_err(|e| e.to_string())
}

/// Send a prompt to multiple models and get responses
#[tauri::command]
pub async fn send_prompt(
    session_id: String,
    request: PromptRequest,
    state: State<'_, AppState>,
) -> Result<PromptTile, String> {
    let tile_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();

    // Create initial responses map
    let mut responses: HashMap<String, ModelResponse> = HashMap::new();
    for model_id in &request.models {
        let model_name = model_id.split('/').last().unwrap_or(model_id).to_string();
        responses.insert(
            model_id.clone(),
            ModelResponse {
                id: uuid::Uuid::new_v4().to_string(),
                model_id: model_id.clone(),
                model_name,
                content: String::new(),
                status: ResponseStatus::Pending,
                error: None,
                tokens_used: None,
                created_at: now,
            },
        );
    }

    // Create the tile
    let mut tile = PromptTile {
        id: tile_id.clone(),
        prompt: request.prompt.clone(),
        system_prompt: request.system_prompt.clone(),
        models: request.models.clone(),
        responses: responses.clone(),
        position: request.position.unwrap_or_default(),
        created_at: now,
        context_mode: request.context_mode,
        parent_tile_id: request.parent_tile_id,
    };

    // Add tile to session
    {
        let mut store = state.canvas_store.write().await;
        store.add_tile(&session_id, tile.clone()).map_err(|e| e.to_string())?;
    }

    // Send requests to each model (sequentially for simplicity)
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: request.prompt.clone(),
    }];

    for model_id in &request.models {
        // Update status to streaming
        if let Some(response) = tile.responses.get_mut(model_id) {
            response.status = ResponseStatus::Streaming;
        }

        // Send request
        let openrouter = state.openrouter.read().await;
        let result = openrouter
            .chat(model_id, messages.clone(), request.system_prompt.as_deref())
            .await;
        drop(openrouter); // Release lock before updating store

        // Update response
        if let Some(response) = tile.responses.get_mut(model_id) {
            match result {
                Ok(content) => {
                    response.content = content;
                    response.status = ResponseStatus::Completed;
                }
                Err(e) => {
                    response.error = Some(e.to_string());
                    response.status = ResponseStatus::Error;
                }
            }
        }

        // Save updated tile to session
        {
            let mut store = state.canvas_store.write().await;
            if let Ok(mut session) = store.get_session(&session_id) {
                if let Some(t) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
                    *t = tile.clone();
                }
                session.updated_at = Utc::now();
                let _ = store.update_session(
                    &session_id,
                    SessionUpdate {
                        title: Some(session.title),
                        description: session.description,
                        tags: Some(session.tags),
                        status: Some(session.status),
                        viewport: Some(session.viewport),
                    },
                );
            }
        }
    }

    Ok(tile)
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

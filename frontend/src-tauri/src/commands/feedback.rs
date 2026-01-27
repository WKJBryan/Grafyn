//! Tauri commands for feedback submission

use crate::models::feedback::{FeedbackCreate, FeedbackResponse, FeedbackStatus, SystemInfo, PendingFeedback};
use crate::AppState;
use tauri::State;

/// Submit feedback (creates GitHub issue or queues if offline)
#[tauri::command]
pub async fn submit_feedback(
    feedback: FeedbackCreate,
    state: State<'_, AppState>,
) -> Result<FeedbackResponse, String> {
    let service = state.feedback_service.read().await;
    service
        .submit(feedback)
        .await
        .map_err(|e| e.to_string())
}

/// Get system information for the feedback form
#[tauri::command]
pub async fn get_system_info(
    current_page: Option<String>,
    state: State<'_, AppState>,
) -> Result<SystemInfo, String> {
    let service = state.feedback_service.read().await;
    Ok(service.get_system_info(current_page))
}

/// Get the feedback service status
#[tauri::command]
pub async fn feedback_status(state: State<'_, AppState>) -> Result<FeedbackStatus, String> {
    let service = state.feedback_service.read().await;
    Ok(service.get_status())
}

/// Get all pending feedback items (queued for later submission)
#[tauri::command]
pub async fn get_pending_feedback(
    state: State<'_, AppState>,
) -> Result<Vec<PendingFeedback>, String> {
    let service = state.feedback_service.read().await;
    service.get_pending().map_err(|e| e.to_string())
}

/// Retry submitting all pending feedback items
#[tauri::command]
pub async fn retry_pending_feedback(
    state: State<'_, AppState>,
) -> Result<Vec<FeedbackResponse>, String> {
    let service = state.feedback_service.read().await;
    service
        .retry_pending()
        .await
        .map_err(|e| e.to_string())
}

/// Clear a specific pending feedback item
#[tauri::command]
pub async fn clear_pending_feedback(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let service = state.feedback_service.read().await;
    service.clear_pending(&id).map_err(|e| e.to_string())
}

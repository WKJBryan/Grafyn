use crate::{
    services::{
        evidence::*,
        evidence_bridge::{self as bridge, EvidenceLocation},
        evidence_prediction,
    },
    AppState,
};
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

async fn ollama_base_url(state: &AppState) -> Result<String, String> {
    let ollama = state.ollama_service()?;
    let guard = ollama.read().await;
    Ok(guard.base_url().to_string())
}

pub(crate) async fn location(state: &AppState, scope: &str) -> Result<EvidenceLocation, String> {
    let settings = state.settings_service.read().await.get().clone();
    match scope {
        "pilot" => Ok(bridge::pilot_location(&settings.effective_data_path())),
        "current" => {
            let store = state.knowledge_store.read().await;
            let notes = store.list_full_notes().map_err(|e| e.to_string())?;
            let root = crate::models::settings::twin_data_path_for_vault(
                &store.data_path(),
                store.vault_path(),
            )
            .map_err(|e| e.to_string())?;
            bridge::current_location(&root, &notes).map_err(|e| e.to_string())
        }
        _ => Err("Unknown evidence vault scope".into()),
    }
}

pub(crate) async fn reconcile_current(state: &AppState) -> Result<EvidenceSnapshot, String> {
    let mut store = state.knowledge_store.write().await;
    store.refresh_cache();
    let loc = bridge::reconcile_knowledge_store(&store).map_err(|e| e.to_string())?;
    bridge::transaction(&loc, |s| s.snapshot()).map_err(|e| e.to_string())
}

pub(crate) async fn current_context(
    state: &AppState,
    query: &str,
) -> Result<ContextPacket, String> {
    reconcile_current(state).await?;
    let loc = location(state, "current").await?;
    bridge::shared_context(&loc, query).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn evidence_snapshot(
    scope: String,
    state: State<'_, AppState>,
) -> Result<EvidenceSnapshot, String> {
    if scope == "current" {
        return reconcile_current(state.inner()).await;
    }
    let loc = location(state.inner(), &scope).await?;
    bridge::transaction(&loc, |s| s.snapshot()).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn create_evidence_pilot(
    scope: String,
    state: State<'_, AppState>,
) -> Result<EvidenceSnapshot, String> {
    if scope != "pilot" {
        return Err("Pilot creation requires pilot scope".into());
    }
    let settings = state.settings_service.read().await.get().clone();
    let base = settings.effective_data_path().join("pilots/bryan");
    std::fs::create_dir_all(base.join("vault")).map_err(|e| e.to_string())?;
    let loc = location(state.inner(), &scope).await?;
    bridge::transaction(&loc, |s| s.reconcile_sources(Vec::new())).map_err(|e| e.to_string())
}
#[derive(Deserialize)]
pub struct InterviewRequest {
    draft: InterviewDraft,
    submit: bool,
}
#[tauri::command]
pub async fn save_evidence_interview(
    scope: String,
    request: InterviewRequest,
    state: State<'_, AppState>,
) -> Result<EvidenceSnapshot, String> {
    let loc = location(state.inner(), &scope).await?;
    bridge::transaction(&loc, |s| s.save_interview(request.draft, request.submit))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn save_evidence_goal(
    scope: String,
    request: GoalInput,
    state: State<'_, AppState>,
) -> Result<GoalRevision, String> {
    let loc = location(state.inner(), &scope).await?;
    bridge::transaction(&loc, |s| s.save_goal(request)).map_err(|e| e.to_string())
}
#[derive(Deserialize)]
pub struct RelationshipRequest {
    id: String,
    status: ReviewStatus,
    relation: Option<RelationshipKind>,
}
#[tauri::command]
pub async fn review_evidence_relationship(
    scope: String,
    request: RelationshipRequest,
    state: State<'_, AppState>,
) -> Result<Relationship, String> {
    let loc = location(state.inner(), &scope).await?;
    bridge::transaction(&loc, |s| {
        s.review_relationship(&request.id, request.status, request.relation)
    })
    .map_err(|e| e.to_string())
}
#[derive(Deserialize)]
pub struct StatementReviewRequest {
    id: String,
    status: ReviewStatus,
}
#[tauri::command]
pub async fn review_evidence_statement(
    scope: String,
    request: StatementReviewRequest,
    state: State<'_, AppState>,
) -> Result<PersonalEvidence, String> {
    let loc = location(state.inner(), &scope).await?;
    bridge::transaction(&loc, |s| s.review_statement(&request.id, request.status))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn process_evidence_jobs(
    scope: String,
    state: State<'_, AppState>,
) -> Result<EvidenceSnapshot, String> {
    if scope == "current" {
        reconcile_current(state.inner()).await?;
    }
    let loc = location(state.inner(), &scope).await?;
    let url = ollama_base_url(state.inner()).await?;
    bridge::process_one(&loc, &url)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn predict_evidence_decision(
    scope: String,
    request: evidence_prediction::PredictionRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if scope == "current" {
        reconcile_current(state.inner()).await?;
    }
    let loc = location(state.inner(), &scope).await?;
    let url = ollama_base_url(state.inner()).await?;
    evidence_prediction::predict(&loc, &url, request)
        .await
        .map_err(|e| e.to_string())
}

/// Explicit setup action invoked by the user's install button, never by the worker.
#[tauri::command]
pub async fn install_evidence_embeddings(
    scope: String,
    state: State<'_, AppState>,
) -> Result<EvidenceSnapshot, String> {
    let loc = location(state.inner(), &scope).await?;
    let url = ollama_base_url(state.inner()).await?;
    let parsed = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    ) {
        return Err("Embedding setup requires local Ollama".into());
    }
    let result: Value = reqwest::Client::new()
        .post(format!("{}/api/pull", url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(900))
        .json(&serde_json::json!({"model":"embeddinggemma","stream":false}))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if result.get("error").is_some() {
        return Err(format!(
            "Embedding installation failed: {}",
            result["error"]
        ));
    }
    bridge::process_one(&loc, &url)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn record_evidence_choice(
    scope: String,
    request: evidence_prediction::ChoiceRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let loc = location(state.inner(), &scope).await?;
    evidence_prediction::record_choice(&loc, request).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn list_evidence_predictions(
    scope: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let loc = location(state.inner(), &scope).await?;
    evidence_prediction::list_predictions(&loc).map_err(|e| e.to_string())
}

/// Polling also notices external edits and MCP imports. No provider lock spans a request.
pub(crate) fn start_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(error) = reconcile_current(&state).await {
                log::warn!("Evidence reconciliation: {}", error);
            }
            let Ok(url) = ollama_base_url(&state).await else {
                continue;
            };
            for scope in ["current", "pilot"] {
                if let Ok(loc) = location(&state, scope).await {
                    if scope == "pilot" && !loc.root.join("evidence.json").exists() {
                        continue;
                    }
                    if let Err(error) = bridge::process_one(&loc, &url).await {
                        log::warn!("Evidence worker: {}", error);
                    }
                }
            }
        }
    });
}

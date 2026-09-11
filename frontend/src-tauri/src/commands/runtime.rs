use crate::models::runtime::RuntimeStatusV1;
use crate::AppState;
use tauri::State;

pub(crate) async fn runtime_status_inner(
    base_status: &RuntimeStatusV1,
    state: &AppState,
) -> RuntimeStatusV1 {
    let canonical_runtime_available =
        state.mutation_coordinator.is_some() && state.mutation_startup_error.read().await.is_none();
    if canonical_runtime_available {
        base_status.clone()
    } else {
        crate::app_runtime::mask_canonical_runtime_unavailable(base_status.clone())
    }
}

#[tauri::command]
pub async fn get_runtime_status(
    status: State<'_, RuntimeStatusV1>,
    state: State<'_, AppState>,
) -> Result<RuntimeStatusV1, String> {
    Ok(runtime_status_inner(status.inner(), state.inner()).await)
}

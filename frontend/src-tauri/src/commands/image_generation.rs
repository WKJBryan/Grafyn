use crate::models::image_generation::{
    DiscardGeneratedImageReceiptRequest, DiscoverImageModelsRequest, ExportGeneratedImageRequest,
    GenerateImageRequest, GeneratedImageExportResult, GeneratedImagePreview,
    GeneratedImageSyncDisposition, GeneratedImageSyncStatus, ImageGenerationModel,
    ImageModelCapabilityRequest, ImageModelCapabilityResponse, LoadGeneratedImageRequest,
    LoadedGeneratedImage, SaveGeneratedImageRequest, SavedGeneratedImage,
};
use crate::models::twin_event::{ContentDigest, TwinEventPayload};
#[cfg(any(target_os = "android", test))]
use crate::services::android_bridge::ShareDescriptor;
use crate::services::attachment_store::{
    prepare_generated_image_for_storage, validate_generated_image_bytes,
};
use crate::AppState;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use std::path::Path;
#[cfg(target_os = "android")]
use tauri::Manager;
use tauri::State;
#[cfg(desktop)]
use tauri_plugin_dialog::DialogExt;

#[cfg(any(mobile, test))]
const MOBILE_IMAGE_SHARE_UNAVAILABLE: &str =
    "Generated image file sharing is unavailable on mobile until secure native sharing is enabled";

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImageShareResult {
    pub share_sheet_opened: bool,
}

#[cfg(test)]
static SAVE_ENTRY_PAUSE: std::sync::OnceLock<
    std::sync::Mutex<
        Option<(
            String,
            std::sync::Arc<std::sync::Barrier>,
            std::sync::Arc<std::sync::Barrier>,
        )>,
    >,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn pause_save_after_root_acquisition_once(
    receipt_id: String,
    entered: std::sync::Arc<std::sync::Barrier>,
    resume: std::sync::Arc<std::sync::Barrier>,
) {
    *SAVE_ENTRY_PAUSE
        .get_or_init(Default::default)
        .lock()
        .unwrap() = Some((receipt_id, entered, resume));
}

#[cfg(test)]
fn run_save_entry_pause(receipt_id: &str) {
    let pause = SAVE_ENTRY_PAUSE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .take();
    if let Some((expected, entered, resume)) = pause {
        if expected == receipt_id {
            entered.wait();
            resume.wait();
        } else {
            *SAVE_ENTRY_PAUSE.get().unwrap().lock().unwrap() = Some((expected, entered, resume));
        }
    }
}

#[cfg(test)]
static POSTCOMMIT_RESPONSE_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<(String, String)>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn fail_postcommit_response_once(receipt_id: &str, message: &str) {
    *POSTCOMMIT_RESPONSE_FAILURE
        .get_or_init(Default::default)
        .lock()
        .unwrap() = Some((receipt_id.to_owned(), message.to_owned()));
}

#[cfg(test)]
fn take_postcommit_response_failure(receipt_id: &str) -> Option<String> {
    let mut failure = POSTCOMMIT_RESPONSE_FAILURE
        .get_or_init(Default::default)
        .lock()
        .unwrap();
    if failure
        .as_ref()
        .is_some_and(|(expected, _)| expected == receipt_id)
    {
        failure.take().map(|(_, message)| message)
    } else {
        None
    }
}

pub(crate) async fn discover_image_models_inner(
    state: &AppState,
    _request: DiscoverImageModelsRequest,
) -> Result<Vec<ImageGenerationModel>, String> {
    let service = state.openrouter.read().await.clone();
    service
        .discover_image_models()
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn image_model_capability_inner(
    state: &AppState,
    request: ImageModelCapabilityRequest,
) -> Result<ImageModelCapabilityResponse, String> {
    request.validate()?;
    let service = state.openrouter.read().await.clone();
    service
        .image_model_capability(&request.model_id)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn generate_image_inner(
    state: &AppState,
    request: GenerateImageRequest,
) -> Result<GeneratedImagePreview, String> {
    request.validate()?;
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = state;
        let _ = request;
        return Err(
            "Image generation is unavailable on mobile until secure API-key storage is ready"
                .into(),
        );
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let root_ticket = crate::commands::acquire_root_epoch(state).await?;
        let vault_scope = root_ticket.authority().root_scope.clone();
        root_ticket.finish(state).await?;
        let service = state.openrouter.read().await.clone();
        service
            .generate_image(request, vault_scope)
            .await
            .map_err(|error| error.to_string())
    }
}

pub(crate) async fn discard_generated_image_receipt_inner(
    state: &AppState,
    request: DiscardGeneratedImageReceiptRequest,
) -> Result<(), String> {
    request.validate()?;
    state
        .openrouter
        .read()
        .await
        .consume_generated_image(&request.receipt_id);
    Ok(())
}

pub(crate) async fn save_generated_image_inner(
    state: &AppState,
    request: SaveGeneratedImageRequest,
    captured_at: DateTime<Utc>,
) -> Result<SavedGeneratedImage, String> {
    request.validate()?;
    let root_ticket = crate::commands::acquire_root_epoch(state).await?;
    #[cfg(test)]
    run_save_entry_pause(&request.receipt_id);
    let expected = root_ticket.authority().clone();
    let service = state.openrouter.read().await.clone();
    let receipt = service
        .lease_generated_image(&request.receipt_id)
        .map_err(|error| error.to_string())?;
    if receipt.vault_scope != expected.root_scope {
        service.release_generated_image(&request.receipt_id);
        return Err(
            "Generated image preview belongs to a different vault; switch back before saving"
                .into(),
        );
    }
    let prepared = match prepare_generated_image_for_storage(
        &receipt.bytes,
        Some(&receipt.media_type),
        request.retention_policy,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            service.release_generated_image(&request.receipt_id);
            return Err(error.to_string());
        }
    };
    let sync_engine = match state.sync_engine.as_ref() {
        Some(sync_engine) => sync_engine,
        None => {
            service.release_generated_image(&request.receipt_id);
            return Err("Image attachment storage is unavailable".into());
        }
    };
    let prepared_digest = crate::models::attachment::sha256_attachment_digest(prepared.bytes());
    let attachment_digest = match ContentDigest::parse(prepared_digest.to_string()) {
        Ok(digest) => digest,
        Err(error) => {
            service.release_generated_image(&request.receipt_id);
            return Err(error);
        }
    };
    let (_pending_image, registered_digest) = match sync_engine.register_pending_generated_image(
        &expected.root_scope,
        prepared.bytes(),
        prepared.media_type(),
    ) {
        Ok(registered) => registered,
        Err(error) => {
            service.release_generated_image(&request.receipt_id);
            return Err(error.to_string());
        }
    };
    if registered_digest != prepared_digest {
        service.release_generated_image(&request.receipt_id);
        return Err("Pending generated image digest changed during registration".into());
    }
    let prepared_dimensions = prepared.dimensions();
    let prepared_media_type = prepared.media_type().to_string();
    let prepared_size = prepared.bytes().len() as u64;
    let (note_create, observation) = match crate::commands::twin_state::plan_generated_image_capture(
        crate::commands::twin_state::GeneratedImageCapturePlanInput {
            prompt: receipt.prompt,
            annotation: request.annotation.clone(),
            model_id: receipt.model_id,
            gateway: receipt.gateway,
            provider_tag: receipt.provider_tag,
            resolution: receipt.resolution,
            aspect_ratio: receipt.aspect_ratio,
            attachment_digest: attachment_digest.clone(),
            media_type: prepared_media_type.clone(),
            byte_size: prepared_size,
            width: prepared_dimensions.0,
            height: prepared_dimensions.1,
            retention_policy: request.retention_policy,
            grafyn_sync: request.grafyn_sync,
        },
        captured_at,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            service.release_generated_image(&request.receipt_id);
            return Err(error);
        }
    };
    let persistence = {
        let mut store = state.knowledge_store.write().await;
        store.create_generated_image_capture_expecting_authority(
            note_create,
            observation,
            expected.clone(),
        )
    };
    let retry_safe_failure = persistence.is_proven_precommit();
    let mutation = persistence.into_result();
    let completed = match crate::commands::complete_knowledge_note_mutation(
        state,
        &expected,
        None,
        mutation,
        "generated image save",
    )
    .await
    {
        Ok(completed) => completed,
        Err(error) => {
            if retry_safe_failure {
                service.release_generated_image(&request.receipt_id);
                return Err(error);
            } else {
                service.consume_generated_image(&request.receipt_id);
            }
            let terminal_error = if error.to_ascii_lowercase().contains("do not retry") {
                error
            } else {
                format!(
                    "{error}; generated image receipt was consumed after an uncertain save; do not retry"
                )
            };
            return Err(terminal_error);
        }
    };
    service.consume_generated_image(&request.receipt_id);
    let response = (|| -> Result<(SavedGeneratedImage, String), String> {
        #[cfg(test)]
        if let Some(error) = take_postcommit_response_failure(&request.receipt_id) {
            return Err(format!(
                "Generated image save committed but response finalization failed; do not retry: {error}"
            ));
        }
        let catalog = sync_engine
            .cataloged_image_expecting_scope(&expected.root_scope, &prepared_digest)
            .map_err(|error| format!("Generated image save committed but its catalog is unavailable; do not retry: {error}"))?
            .ok_or_else(|| {
                "Generated image save committed without publishing its catalog; do not retry".to_string()
            })?;
        if catalog.dimensions() != prepared_dimensions
            || catalog.media_type() != prepared_media_type
            || catalog.decoded_size() != prepared_size
        {
            return Err(
                "Generated image save committed with conflicting catalog metadata; do not retry"
                    .into(),
            );
        }
        let sync_disposition = match request.grafyn_sync {
            crate::models::image_generation::GeneratedImageSyncPolicy::LocalOnly => {
                GeneratedImageSyncDisposition {
                    status: GeneratedImageSyncStatus::LocalOnly,
                    manifest_count: 0,
                    chunk_count: 0,
                    operation_count: 0,
                }
            }
            crate::models::image_generation::GeneratedImageSyncPolicy::Inherit => {
                let mutation_id = completed
                    .commit
                    .as_ref()
                    .and_then(|commit| commit.mutation_id.as_ref())
                    .ok_or_else(|| {
                        "Generated image save committed without its sync mutation identity; do not retry"
                            .to_string()
                    })?;
                match sync_engine
                    .generated_image_outbox_disposition(
                        &expected.root_scope,
                        mutation_id,
                        &prepared_digest,
                    )
                    .map_err(|error| {
                        format!(
                            "Generated image save committed but its exact sync disposition is unavailable; do not retry: {error}"
                        )
                    })? {
                    Some(disposition) => GeneratedImageSyncDisposition {
                        status: GeneratedImageSyncStatus::Queued,
                        manifest_count: disposition.manifest_count,
                        chunk_count: disposition.chunk_count,
                        operation_count: disposition.operation_count,
                    },
                    None => GeneratedImageSyncDisposition {
                        status: GeneratedImageSyncStatus::AwaitingProvisioning,
                        manifest_count: 0,
                        chunk_count: 0,
                        operation_count: 0,
                    },
                }
            }
        };
        let note = completed.note.clone();
        let observation_id = format!("companion-capture-{}", note.id);
        let observation_event_id = completed
            .commit
            .as_ref()
            .and_then(|commit| {
                commit.events.iter().find_map(|event| match &event.payload {
                    TwinEventPayload::ObservationRecorded(observation)
                        if observation.observation_id.as_str() == observation_id =>
                    {
                        Some(event.event_id.clone())
                    }
                    _ => None,
                })
            })
            .ok_or_else(|| {
                "Generated image save committed without its observation identity; do not retry"
                    .to_string()
            })?;
        let note_id = note.id.clone();
        Ok((
            SavedGeneratedImage {
                note,
                observation_event_id,
                attachment_digest: attachment_digest.clone(),
                media_type: catalog.media_type().to_string(),
                byte_size: catalog.decoded_size(),
                width: catalog.dimensions().0,
                height: catalog.dimensions().1,
                sync_disposition,
            },
            note_id,
        ))
    })();
    let mut continuation_authority = completed.continuation_authority.clone();
    let repaired = completed.repaired;
    if let Some(commit) = completed.commit.as_ref() {
        drop(root_ticket);
        if !repaired {
            match crate::commands::repair_after_authority_mutation(
                state,
                commit,
                "generated image save",
            )
            .await
            {
                crate::commands::PostAuthorityRepair::Ready(authority) => {
                    continuation_authority = authority;
                }
                repair => crate::commands::acknowledge_reported_repair(repair),
            }
        }
    } else {
        root_ticket.finish(state).await.map_err(|error| {
            format!(
                "Generated image save committed but root finalization failed; do not retry: {error}"
            )
        })?;
    }
    let (saved, note_id) = response?;
    if let Err(error) = crate::commands::enqueue_vault_optimizer_note_at_authority(
        state,
        &continuation_authority,
        &note_id,
        "image_generation",
    )
    .await
    {
        log::warn!("Generated image save committed but optimizer enqueue failed: {error}");
    }
    Ok(saved)
}

pub(crate) async fn load_generated_image_inner(
    state: &AppState,
    request: LoadGeneratedImageRequest,
) -> Result<LoadedGeneratedImage, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state).await?;
    let sync_engine = state
        .sync_engine
        .as_ref()
        .ok_or_else(|| "Image attachment storage is unavailable".to_string())?;
    let digest = grafyn_sync_protocol::Digest32::parse_hex(request.attachment_digest.as_str())
        .map_err(|error| error.to_string())?;
    let catalog = sync_engine
        .cataloged_image_expecting_scope(&root_ticket.authority().root_scope, &digest)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Generated image attachment is not in this vault".to_string())?;
    let bytes = sync_engine
        .materialized_attachment(&digest)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Generated image attachment blob is missing".to_string())?;
    if crate::models::attachment::sha256_attachment_digest(&bytes) != digest {
        return Err("Generated image attachment digest verification failed".into());
    }
    let validated = validate_generated_image_bytes(&bytes, Some(catalog.media_type()))
        .map_err(|error| error.to_string())?;
    if catalog.decoded_size() != bytes.len() as u64
        || catalog.dimensions() != validated.dimensions()
        || catalog.media_type() != validated.media_type()
    {
        return Err("Generated image attachment catalog does not match its raster".into());
    }
    if let Err(error) = sync_engine
        .generated_image_thumbnail_expecting_scope(&root_ticket.authority().root_scope, &digest)
    {
        log::warn!("Generated image thumbnail cache could not be refreshed: {error}");
    }
    root_ticket.finish(state).await?;
    Ok(LoadedGeneratedImage {
        attachment_digest: request.attachment_digest,
        media_type: catalog.media_type().to_string(),
        base64_data: STANDARD.encode(&bytes),
        byte_size: bytes.len() as u64,
        width: validated.dimensions().0,
        height: validated.dimensions().1,
    })
}

fn write_generated_image_export(
    receipt: &crate::services::openrouter::GeneratedImageReceipt,
    path: &Path,
    retention_policy: crate::models::image_generation::ImageMetadataRetentionPolicy,
) -> Result<GeneratedImageExportResult, String> {
    let prepared = prepare_generated_image_for_storage(
        &receipt.bytes,
        Some(&receipt.media_type),
        retention_policy,
    )
    .map_err(|error| error.to_string())?;
    let validated = validate_generated_image_bytes(prepared.bytes(), Some(prepared.media_type()))
        .map_err(|error| error.to_string())?;
    if validated.dimensions() != (receipt.width, receipt.height) {
        return Err("Generated image receipt dimensions do not match its raster".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "Generated image export requires a matching file extension".to_string())?;
    let extension_matches = match prepared.media_type() {
        "image/png" => extension == "png",
        "image/jpeg" => matches!(extension.as_str(), "jpg" | "jpeg"),
        "image/webp" => extension == "webp",
        _ => false,
    };
    if !extension_matches {
        return Err("Generated image export extension must match its exact MIME".into());
    }
    crate::services::atomic_io::write_atomic(path, prepared.bytes())
        .map_err(|error| format!("Failed to export generated image: {error}"))?;
    Ok(GeneratedImageExportResult { exported: true })
}

#[cfg(test)]
fn export_generated_image_to_path_for_test(
    service: &crate::services::openrouter::OpenRouterService,
    request: ExportGeneratedImageRequest,
    path: &Path,
) -> Result<GeneratedImageExportResult, String> {
    request.validate()?;
    let receipt = service
        .lease_generated_image(&request.receipt_id)
        .map_err(|error| error.to_string())?;
    let result = write_generated_image_export(&receipt, path, request.retention_policy);
    service.release_generated_image(&request.receipt_id);
    result
}

#[cfg(any(target_os = "android", test))]
fn prepare_generated_image_share(
    receipt: &crate::services::openrouter::GeneratedImageReceipt,
    retention_policy: crate::models::image_generation::ImageMetadataRetentionPolicy,
) -> Result<
    (
        ShareDescriptor,
        crate::services::attachment_store::PreparedGeneratedImage,
    ),
    String,
> {
    let prepared = prepare_generated_image_for_storage(
        &receipt.bytes,
        Some(&receipt.media_type),
        retention_policy,
    )
    .map_err(|error| error.to_string())?;
    let validated = validate_generated_image_bytes(prepared.bytes(), Some(prepared.media_type()))
        .map_err(|error| error.to_string())?;
    if validated.dimensions() != (receipt.width, receipt.height) {
        return Err("Generated image receipt dimensions do not match its raster".into());
    }
    let extension = match prepared.media_type() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => return Err("Generated image receipt MIME is unsupported".into()),
    };
    let file_name = format!("grafyn-{}.{}", uuid::Uuid::new_v4(), extension);
    let descriptor = ShareDescriptor::parse(&file_name, prepared.media_type())
        .map_err(|error| error.to_string())?;
    Ok((descriptor, prepared))
}

#[cfg(any(target_os = "android", test))]
async fn share_generated_image_inner(
    state: &AppState,
    request: ExportGeneratedImageRequest,
    share: impl FnOnce(&ShareDescriptor, &[u8]) -> Result<(), String>,
) -> Result<GeneratedImageShareResult, String> {
    request.validate()?;
    let service = state.openrouter.read().await.clone();
    let receipt = service
        .lease_generated_image(&request.receipt_id)
        .map_err(|error| error.to_string())?;
    let result = prepare_generated_image_share(&receipt, request.retention_policy).and_then(
        |(descriptor, prepared)| {
            share(&descriptor, prepared.bytes())?;
            Ok(GeneratedImageShareResult {
                share_sheet_opened: true,
            })
        },
    );
    service.release_generated_image(&request.receipt_id);
    result
}

#[tauri::command]
pub async fn discover_image_models(
    state: State<'_, AppState>,
    request: DiscoverImageModelsRequest,
) -> Result<Vec<ImageGenerationModel>, String> {
    discover_image_models_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn get_image_model_capability(
    state: State<'_, AppState>,
    request: ImageModelCapabilityRequest,
) -> Result<ImageModelCapabilityResponse, String> {
    image_model_capability_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    request: GenerateImageRequest,
) -> Result<GeneratedImagePreview, String> {
    generate_image_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn discard_generated_image_receipt(
    state: State<'_, AppState>,
    request: DiscardGeneratedImageReceiptRequest,
) -> Result<(), String> {
    discard_generated_image_receipt_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn export_generated_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: ExportGeneratedImageRequest,
) -> Result<GeneratedImageExportResult, String> {
    request.validate()?;
    #[cfg(desktop)]
    {
        let service = state.openrouter.read().await.clone();
        let receipt = service
            .lease_generated_image(&request.receipt_id)
            .map_err(|error| error.to_string())?;
        let (extension, label) = match receipt.media_type.as_str() {
            "image/png" => ("png", "PNG image"),
            "image/jpeg" => ("jpg", "JPEG image"),
            "image/webp" => ("webp", "WebP image"),
            _ => {
                service.release_generated_image(&request.receipt_id);
                return Err("Generated image receipt MIME is unsupported".into());
            }
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title("Save generated image")
            .set_file_name(format!("grafyn-generated-image.{extension}"))
            .add_filter(label, &[extension])
            .save_file(move |path| {
                let _ = sender.send(path);
            });
        let selected = match receiver.await {
            Ok(path) => path,
            Err(error) => {
                service.release_generated_image(&request.receipt_id);
                return Err(format!("Generated image Save As dialog failed: {error}"));
            }
        };
        let Some(selected) = selected else {
            service.release_generated_image(&request.receipt_id);
            return Ok(GeneratedImageExportResult { exported: false });
        };
        let path = match selected.into_path() {
            Ok(path) => path,
            Err(error) => {
                service.release_generated_image(&request.receipt_id);
                return Err(format!("Generated image Save As path is invalid: {error}"));
            }
        };
        let result = write_generated_image_export(&receipt, &path, request.retention_policy);
        service.release_generated_image(&request.receipt_id);
        return result;
    }
    #[cfg(mobile)]
    {
        let _ = app;
        let _ = state;
        Err(MOBILE_IMAGE_SHARE_UNAVAILABLE.into())
    }
}

#[cfg(target_os = "android")]
#[tauri::command]
pub async fn share_generated_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: ExportGeneratedImageRequest,
) -> Result<GeneratedImageShareResult, String> {
    share_generated_image_inner(state.inner(), request, move |descriptor, bytes| {
        app.state::<crate::services::android_bridge::AndroidBridge<tauri::Wry>>()
            .stage_and_share_generated_image(descriptor.file_name(), descriptor.mime(), bytes)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn save_generated_image(
    state: State<'_, AppState>,
    request: SaveGeneratedImageRequest,
) -> Result<SavedGeneratedImage, String> {
    save_generated_image_inner(state.inner(), request, Utc::now()).await
}

#[tauri::command]
pub async fn load_generated_image(
    state: State<'_, AppState>,
    request: LoadGeneratedImageRequest,
) -> Result<LoadedGeneratedImage, String> {
    load_generated_image_inner(state.inner(), request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::image_generation::{GeneratedImageSyncPolicy, ImageMetadataRetentionPolicy};
    use crate::models::twin_event::{CausalStream, EvidenceType};
    use crate::services::openrouter::{GeneratedImageReceipt, OpenRouterService};
    use crate::services::sync::engine::SyncEngine;
    use crate::services::sync::identity::VaultIdentity;
    use crate::services::sync::secrets::MemorySecretStore;
    use chrono::TimeZone;
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn image_test_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let (mut state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
        let authority = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_authority_token()
            .unwrap();
        let identity = VaultIdentity {
            descriptor: crate::models::sync::VaultDescriptorV1::generate(),
            root_scope: authority.root_scope,
        };
        let sync_engine = Arc::new(
            SyncEngine::open_core(
                data.path(),
                vault.path(),
                identity,
                None,
                Arc::new(MemorySecretStore::default()),
                state.twin_event_store.clone(),
            )
            .unwrap(),
        );
        let coordinator = Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data.path(),
                vault.path(),
                state.twin_event_store.clone(),
                sync_engine.clone(),
            )
            .unwrap(),
        );
        state.knowledge_store = Arc::new(RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                coordinator.current_namespace_path().unwrap(),
                coordinator.clone(),
            ),
        ));
        state.mutation_coordinator = Some(coordinator);
        state.sync_engine = Some(sync_engine);
        (state, vault, data)
    }

    fn generated_png() -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(2, 2);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn png_with_text_metadata() -> Vec<u8> {
        let mut bytes = generated_png();
        let iend_offset = bytes.len() - 12;
        let payload = b"Comment\0private-export-metadata";
        let mut crc_input = b"tEXt".to_vec();
        crc_input.extend_from_slice(payload);
        let mut crc = 0xffff_ffffu32;
        for byte in crc_input {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(payload);
        chunk.extend_from_slice(&(!crc).to_be_bytes());
        bytes.splice(iend_offset..iend_offset, chunk);
        bytes
    }

    fn install_receipt(state: &mut AppState, prompt: &str) -> String {
        install_receipt_with_bytes(state, prompt, generated_png())
    }

    fn install_receipt_with_bytes(state: &mut AppState, prompt: &str, bytes: Vec<u8>) -> String {
        let service = OpenRouterService::new(String::new());
        let vault_scope = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_authority_token()
            .unwrap()
            .root_scope;
        let receipt_id = service
            .insert_generated_image_for_tests(GeneratedImageReceipt {
                bytes,
                media_type: "image/png".into(),
                width: 2,
                height: 2,
                prompt: prompt.into(),
                model_id: "author/model".into(),
                resolution: "1024x1024".into(),
                aspect_ratio: "1:1".into(),
                vault_scope,
                gateway: "openrouter".into(),
                provider_tag: "test-provider".into(),
            })
            .unwrap();
        state.openrouter = Arc::new(RwLock::new(service));
        receipt_id
    }

    fn local_only_save_request(receipt_id: String) -> SaveGeneratedImageRequest {
        SaveGeneratedImageRequest {
            receipt_id,
            annotation: None,
            retention_policy: ImageMetadataRetentionPolicy::StripMetadata,
            grafyn_sync: GeneratedImageSyncPolicy::LocalOnly,
        }
    }

    fn generated_digest() -> grafyn_sync_protocol::Digest32 {
        crate::models::attachment::sha256_attachment_digest(&generated_png())
    }

    #[test]
    fn image_save_inner_has_a_clock_injected_test_seam() {
        let _ = save_generated_image_inner;
    }

    #[tokio::test]
    async fn local_only_save_uses_receipt_prompt_persists_image_and_consumes_one_shot() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_prompt = "Receipt-owned quiet workspace";
        let receipt_id = install_receipt(&mut state, receipt_prompt);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 0, 0).unwrap();
        let request = SaveGeneratedImageRequest {
            receipt_id: receipt_id.clone(),
            annotation: Some("Canvas composition study".into()),
            retention_policy: ImageMetadataRetentionPolicy::StripMetadata,
            grafyn_sync: GeneratedImageSyncPolicy::LocalOnly,
        };

        let saved = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap();

        assert_eq!(saved.note.title, receipt_prompt);
        assert_eq!(
            saved.note.properties.get("generation_prompt"),
            Some(&serde_json::json!(receipt_prompt))
        );
        assert_eq!(
            saved.note.properties.get("capture_kind"),
            Some(&serde_json::json!("image"))
        );
        assert_eq!(
            saved.note.properties.get("image_metadata_retention"),
            Some(&serde_json::json!("strip_metadata"))
        );
        assert_eq!(
            saved.note.properties.get("generation_gateway"),
            Some(&serde_json::json!("openrouter"))
        );
        assert_eq!(
            saved.note.properties.get("generation_provider"),
            Some(&serde_json::json!("test-provider"))
        );
        assert_eq!(
            serde_json::to_value(&saved).unwrap()["syncDisposition"],
            serde_json::json!({
                "status": "local_only",
                "manifestCount": 0,
                "chunkCount": 0,
                "operationCount": 0
            })
        );
        assert!(saved.note.content.contains(receipt_prompt));
        assert!(!saved.note.properties.contains_key("provider"));
        assert!(!saved.note.properties.contains_key("cost"));
        let events = state.twin_event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| {
            event.causal_stream == CausalStream::LocalOnly
                && event.context.source_channel.as_str() == "image_generation"
        }));
        let TwinEventPayload::ObservationRecorded(observation) = &events[1].payload else {
            panic!("second image event must be the observation");
        };
        assert!(observation.claims.is_empty());
        assert_eq!(
            events[1]
                .evidence
                .iter()
                .filter(|evidence| evidence.evidence_type == EvidenceType::Attachment)
                .count(),
            1
        );
        assert_eq!(
            state
                .sync_engine
                .as_ref()
                .unwrap()
                .status()
                .unwrap()
                .outbox_operations,
            0
        );

        let loaded = load_generated_image_inner(
            &state,
            LoadGeneratedImageRequest {
                attachment_digest: saved.attachment_digest.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(loaded.media_type, "image/png");
        assert_eq!(loaded.width, 2);
        assert_eq!(loaded.height, 2);
        let loaded_bytes = STANDARD.decode(loaded.base64_data).unwrap();
        validate_generated_image_bytes(&loaded_bytes, Some("image/png")).unwrap();
        let thumbnail = state
            .sync_engine
            .as_ref()
            .unwrap()
            .generated_image_thumbnail_expecting_scope(
                &state
                    .mutation_coordinator
                    .as_ref()
                    .unwrap()
                    .current_authority_token()
                    .unwrap()
                    .root_scope,
                &grafyn_sync_protocol::Digest32::parse_hex(saved.attachment_digest.as_str())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            validate_generated_image_bytes(&thumbnail, Some("image/png"))
                .unwrap()
                .dimensions(),
            (2, 2)
        );

        let error = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();
        assert!(error.contains("missing or expired"));
    }

    #[tokio::test]
    async fn discard_receipt_is_idempotent_and_terminally_invalidates_backend_bytes() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Discarded private prompt");
        let request = DiscardGeneratedImageReceiptRequest {
            receipt_id: receipt_id.clone(),
        };

        discard_generated_image_receipt_inner(&state, request.clone())
            .await
            .unwrap();
        discard_generated_image_receipt_inner(&state, request)
            .await
            .unwrap();

        let service = state.openrouter.read().await.clone();
        assert!(service.lease_generated_image(&receipt_id).is_err());
    }

    #[tokio::test]
    async fn inherited_unprovisioned_save_reports_exact_awaiting_disposition() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Awaiting sync image prompt");
        let saved = save_generated_image_inner(
            &state,
            SaveGeneratedImageRequest {
                receipt_id,
                annotation: None,
                retention_policy: ImageMetadataRetentionPolicy::StripMetadata,
                grafyn_sync: GeneratedImageSyncPolicy::Inherit,
            },
            Utc.with_ymd_and_hms(2026, 9, 1, 8, 1, 0).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            serde_json::to_value(saved.sync_disposition).unwrap(),
            serde_json::json!({
                "status": "awaiting_provisioning",
                "manifestCount": 0,
                "chunkCount": 0,
                "operationCount": 0
            })
        );
        assert_eq!(
            state
                .sync_engine
                .as_ref()
                .unwrap()
                .status()
                .unwrap()
                .outbox_operations,
            0
        );
    }

    #[tokio::test]
    async fn receipt_from_another_vault_is_rejected_without_consuming_it() {
        let (mut state, _vault, _data) = image_test_state();
        let service = OpenRouterService::new(String::new());
        let receipt_id = service
            .insert_generated_image_for_tests(GeneratedImageReceipt {
                bytes: generated_png(),
                media_type: "image/png".into(),
                width: 2,
                height: 2,
                prompt: "Foreign vault prompt".into(),
                model_id: "author/model".into(),
                resolution: "1024x1024".into(),
                aspect_ratio: "1:1".into(),
                vault_scope: ContentDigest::parse("ef".repeat(32)).unwrap(),
                gateway: "openrouter".into(),
                provider_tag: "test-provider".into(),
            })
            .unwrap();
        state.openrouter = Arc::new(RwLock::new(service));
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 2, 0).unwrap();

        let first = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap_err();
        let second = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();
        assert!(first.contains("different vault"));
        assert_eq!(second, first);
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unclassified_io_save_failure_terminalizes_receipt_fail_closed() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Retryable receipt prompt");
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 5, 0).unwrap();
        state.mutation_coordinator.as_ref().unwrap().fail_once_at(
            crate::services::twin_events::MutationFaultPoint::BeforePreAuthorityMarker,
        );

        let first = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap_err();
        assert!(first.contains("do not retry"));
        let retry_error = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();

        assert!(retry_error.contains("missing or expired"));
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn proven_precommit_raster_validation_failure_releases_receipt() {
        let (mut state, _vault, _data) = image_test_state();
        let service = OpenRouterService::new(String::new());
        let vault_scope = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_authority_token()
            .unwrap()
            .root_scope;
        let receipt_id = service
            .insert_generated_image_for_tests(GeneratedImageReceipt {
                bytes: b"not a raster".to_vec(),
                media_type: "image/png".into(),
                width: 2,
                height: 2,
                prompt: "Invalid precommit raster".into(),
                model_id: "author/model".into(),
                resolution: "1024x1024".into(),
                aspect_ratio: "1:1".into(),
                vault_scope,
                gateway: "openrouter".into(),
                provider_tag: "test-provider".into(),
            })
            .unwrap();
        state.openrouter = Arc::new(RwLock::new(service));
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 7, 0).unwrap();

        let first = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap_err();
        let second = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();

        assert!(first.contains("format is unknown"));
        assert_eq!(
            second, first,
            "the receipt must be released for a safe retry"
        );
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn proven_precommit_after_lifecycle_stage_publishes_nothing_and_releases_receipt() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Staged retryable receipt prompt");
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 8, 0).unwrap();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        coordinator
            .reject_once_at(crate::services::twin_events::MutationFaultPoint::AfterLifecycleStage);

        let first_error = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap_err();
        assert!(first_error.contains("injected precommit rejection"));
        let digest = generated_digest();
        let sync = state.sync_engine.as_ref().unwrap();
        assert_eq!(sync.pending_generated_image_count(), 0);
        assert_eq!(sync.generated_image_stage_file_count().unwrap(), 0);
        assert_eq!(sync.materialized_attachment(&digest).unwrap(), None);
        assert_eq!(
            sync.cataloged_image_expecting_scope(
                &coordinator.current_authority_token().unwrap().root_scope,
                &digest,
            )
            .unwrap(),
            None
        );
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());

        let saved = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap();
        assert_eq!(saved.attachment_digest.as_str(), digest.to_string());
        assert!(sync.materialized_attachment(&digest).unwrap().is_some());
        assert!(sync
            .cataloged_image_expecting_scope(
                &coordinator.current_authority_token().unwrap().root_scope,
                &digest,
            )
            .unwrap()
            .is_some());
        assert_eq!(sync.pending_generated_image_count(), 0);
        assert_eq!(sync.generated_image_stage_file_count().unwrap(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authority_pause_never_exposes_catalog_before_note_and_event_commit() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Paused authority receipt prompt");
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 9, 0).unwrap();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        let scope = coordinator.current_authority_token().unwrap().root_scope;
        let entered = Arc::new(std::sync::Barrier::new(2));
        let resume = Arc::new(std::sync::Barrier::new(2));
        coordinator.pause_after_authority_advance_once(entered.clone(), resume.clone());
        let state = Arc::new(state);
        let task_state = state.clone();
        let save = tokio::spawn(async move {
            save_generated_image_inner(&task_state, request, captured_at).await
        });

        entered.wait();
        let digest = generated_digest();
        let sync = state.sync_engine.as_ref().unwrap();
        assert_eq!(
            sync.cataloged_image_expecting_scope(&scope, &digest)
                .unwrap(),
            None
        );
        assert_eq!(sync.materialized_attachment(&digest).unwrap(), None);
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
        assert_eq!(sync.generated_image_stage_file_count().unwrap(), 2);
        resume.wait();

        let saved = save.await.unwrap().unwrap();
        assert_eq!(saved.attachment_digest.as_str(), digest.to_string());
        assert!(sync
            .cataloged_image_expecting_scope(&scope, &digest)
            .unwrap()
            .is_some());
        assert_eq!(sync.generated_image_stage_file_count().unwrap(), 0);
        assert_eq!(sync.pending_generated_image_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn save_entry_root_ticket_prevents_a_click_time_vault_redirect() {
        let (mut state, old_vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Root-bound image prompt");
        let request = local_only_save_request(receipt_id.clone());
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 9, 15).unwrap();
        let replacement_vault = tempfile::tempdir().unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let resume = Arc::new(std::sync::Barrier::new(2));
        pause_save_after_root_acquisition_once(receipt_id, entered.clone(), resume.clone());
        let state = Arc::new(state);
        let save_state = state.clone();
        let save = tokio::spawn(async move {
            save_generated_image_inner(&save_state, request, captured_at).await
        });
        entered.wait();

        let transition = state.vault_transition.clone();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        let replacement_path = replacement_vault.path().to_path_buf();
        let switched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let switch_finished = switched.clone();
        let switch = tokio::spawn(async move {
            let _write = transition.write().await;
            coordinator
                .retarget_markdown_root(&replacement_path)
                .unwrap();
            switch_finished.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(!switched.load(std::sync::atomic::Ordering::SeqCst));
        resume.wait();

        let saved = save.await.unwrap().unwrap();
        switch.await.unwrap();
        assert!(switched.load(std::sync::atomic::Ordering::SeqCst));
        let old_contains_note = walkdir::WalkDir::new(old_vault.path())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .any(|entry| {
                std::fs::read_to_string(entry.path())
                    .is_ok_and(|value| value.contains("Root-bound image prompt"))
            });
        let replacement_contains_note = walkdir::WalkDir::new(replacement_vault.path())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .any(|entry| {
                std::fs::read_to_string(entry.path())
                    .is_ok_and(|value| value.contains("Root-bound image prompt"))
            });
        assert_eq!(saved.note.title, "Root-bound image prompt");
        assert!(old_contains_note);
        assert!(!replacement_contains_note);
    }

    #[tokio::test]
    async fn rejected_duplicate_digest_never_deletes_existing_referenced_catalog() {
        let (mut state, _vault, _data) = image_test_state();
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 9, 30).unwrap();
        let first_receipt = install_receipt(&mut state, "First shared digest prompt");
        let first =
            save_generated_image_inner(&state, local_only_save_request(first_receipt), captured_at)
                .await
                .unwrap();
        let digest = generated_digest();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        let scope = coordinator.current_authority_token().unwrap().root_scope;
        let second_receipt = install_receipt(&mut state, "Second shared digest prompt");
        coordinator
            .reject_once_at(crate::services::twin_events::MutationFaultPoint::AfterLifecycleStage);

        save_generated_image_inner(
            &state,
            local_only_save_request(second_receipt),
            captured_at + chrono::Duration::seconds(1),
        )
        .await
        .unwrap_err();

        assert_eq!(first.attachment_digest.as_str(), digest.to_string());
        assert_eq!(
            state
                .sync_engine
                .as_ref()
                .unwrap()
                .materialized_attachment(&digest)
                .unwrap(),
            Some(generated_png())
        );
        assert!(state
            .sync_engine
            .as_ref()
            .unwrap()
            .cataloged_image_expecting_scope(&scope, &digest)
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn authority_advanced_save_terminalizes_receipt_and_never_duplicates_evidence() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Uncertain receipt prompt");
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 10, 0).unwrap();
        state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterAuthorityAdvance);

        let recovered = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap();
        assert_eq!(recovered.note.title, "Uncertain receipt prompt");
        let second = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();

        assert!(second.contains("missing or expired"));
        assert_eq!(state.twin_event_store.ordered_events().unwrap().len(), 2);
        assert_eq!(
            state
                .sync_engine
                .as_ref()
                .unwrap()
                .generated_image_stage_file_count()
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn postcommit_response_failure_still_repairs_readiness_and_never_retries() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Postcommit response failure prompt");
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 12, 0).unwrap();
        fail_postcommit_response_once(&request.receipt_id, "injected catalog lookup failure");

        let first = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap_err();
        assert!(first.contains("committed"));
        assert!(first.contains("do not retry"));
        let retry = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();

        assert!(retry.contains("missing or expired"));
        assert_eq!(state.twin_event_store.ordered_events().unwrap().len(), 2);
        assert_eq!(
            state
                .knowledge_store
                .read()
                .await
                .list_notes()
                .unwrap()
                .len(),
            1
        );
        let committed_authority = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_authority_token()
            .unwrap();
        assert_eq!(
            state.loaded_authority.read().await.as_ref(),
            Some(&committed_authority)
        );
        state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .require_namespace_ready()
            .unwrap();
    }

    #[tokio::test]
    async fn recovery_pending_save_error_consumes_receipt_and_eventually_recovers_once() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Recovery-pending receipt prompt");
        let request = local_only_save_request(receipt_id);
        let captured_at = Utc.with_ymd_and_hms(2026, 9, 1, 8, 15, 0).unwrap();
        let coordinator = state.mutation_coordinator.as_ref().unwrap();
        coordinator.fail_in_sequence(&[
            crate::services::twin_events::MutationFaultPoint::AfterAuthorityAdvance,
            crate::services::twin_events::MutationFaultPoint::BeforePendingRecovery,
        ]);

        let first_error = save_generated_image_inner(&state, request.clone(), captured_at)
            .await
            .unwrap_err();
        assert!(first_error.contains("do not retry") || first_error.contains("recovery"));
        let retry_error = save_generated_image_inner(&state, request, captured_at)
            .await
            .unwrap_err();
        assert!(retry_error.contains("missing or expired"));

        let sync = state.sync_engine.as_ref().unwrap();
        let before_recovery = sync.materialized_attachment(&generated_digest()).unwrap();
        let before_recovery_events = state.twin_event_store.ordered_events().unwrap().len();
        assert_eq!(before_recovery.is_some(), before_recovery_events == 2);
        assert_eq!(
            sync.generated_image_stage_file_count().unwrap(),
            if before_recovery.is_some() { 0 } else { 2 }
        );
        assert_eq!(sync.pending_generated_image_count(), 0);

        coordinator.recover_pending().unwrap();
        assert_eq!(state.twin_event_store.ordered_events().unwrap().len(), 2);
        assert!(sync
            .materialized_attachment(&generated_digest())
            .unwrap()
            .is_some());
        assert_eq!(sync.generated_image_stage_file_count().unwrap(), 0);
        coordinator.recover_pending().unwrap();
        assert_eq!(state.twin_event_store.ordered_events().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn android_share_strips_metadata_revalidates_and_never_mutates_or_consumes() {
        let (mut state, _vault, _data) = image_test_state();
        let original = png_with_text_metadata();
        let receipt_id =
            install_receipt_with_bytes(&mut state, "Share-only receipt prompt", original);
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));

        for _ in 0..2 {
            let captured = captured.clone();
            let result = share_generated_image_inner(
                &state,
                ExportGeneratedImageRequest {
                    receipt_id: receipt_id.clone(),
                    retention_policy: ImageMetadataRetentionPolicy::StripMetadata,
                },
                move |descriptor, bytes| {
                    captured.lock().unwrap().push((
                        descriptor.file_name().to_owned(),
                        descriptor.mime().to_owned(),
                        bytes.to_vec(),
                    ));
                    Ok(())
                },
            )
            .await
            .unwrap();
            assert!(result.share_sheet_opened);
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                serde_json::json!({ "shareSheetOpened": true })
            );
        }

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert_ne!(captured[0].0, captured[1].0);
        for (file_name, mime, bytes) in captured.iter() {
            assert!(file_name.starts_with("grafyn-"));
            assert!(file_name.ends_with(".png"));
            assert_eq!(mime, "image/png");
            validate_generated_image_bytes(bytes, Some(mime)).unwrap();
            assert!(!bytes
                .windows(b"private-export-metadata".len())
                .any(|window| window == b"private-export-metadata"));
        }
        drop(captured);

        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
        assert!(state
            .knowledge_store
            .read()
            .await
            .list_notes()
            .unwrap()
            .is_empty());
        assert_eq!(
            state
                .sync_engine
                .as_ref()
                .unwrap()
                .status()
                .unwrap()
                .outbox_operations,
            0
        );
        let service = state.openrouter.read().await.clone();
        assert!(service.lease_generated_image(&receipt_id).is_ok());
        service.release_generated_image(&receipt_id);
    }

    #[tokio::test]
    async fn android_share_failure_is_retryable_and_keeps_the_exact_receipt_usable() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Retryable share prompt");
        let request = ExportGeneratedImageRequest {
            receipt_id: receipt_id.clone(),
            retention_policy: ImageMetadataRetentionPolicy::RetainOriginal,
        };

        let error = share_generated_image_inner(&state, request.clone(), |_descriptor, _bytes| {
            Err("android_share_backend_unavailable".to_owned())
        })
        .await
        .unwrap_err();
        assert_eq!(error, "android_share_backend_unavailable");

        let retry = share_generated_image_inner(&state, request, |_descriptor, _bytes| Ok(()))
            .await
            .unwrap();
        assert!(retry.share_sheet_opened);
        let service = state.openrouter.read().await.clone();
        assert!(service.lease_generated_image(&receipt_id).is_ok());
        service.release_generated_image(&receipt_id);
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn desktop_export_writes_only_selected_raster_without_consuming_or_mutating_vault() {
        let (mut state, _vault, _data) = image_test_state();
        let original = png_with_text_metadata();
        let receipt_id =
            install_receipt_with_bytes(&mut state, "Export-only receipt prompt", original.clone());
        let export_dir = tempfile::tempdir().unwrap();
        let output = export_dir.path().join("selected-preview.png");
        let retained_output = export_dir.path().join("selected-preview-original.png");
        let service = state.openrouter.read().await.clone();

        let exported = export_generated_image_to_path_for_test(
            &service,
            ExportGeneratedImageRequest {
                receipt_id: receipt_id.clone(),
                retention_policy: ImageMetadataRetentionPolicy::StripMetadata,
            },
            &output,
        )
        .unwrap();

        assert!(exported.exported);
        let stripped = std::fs::read(output).unwrap();
        assert_ne!(stripped, original);
        assert!(!stripped
            .windows(b"private-export-metadata".len())
            .any(|window| window == b"private-export-metadata"));
        let retained = export_generated_image_to_path_for_test(
            &service,
            ExportGeneratedImageRequest {
                receipt_id: receipt_id.clone(),
                retention_policy: ImageMetadataRetentionPolicy::RetainOriginal,
            },
            &retained_output,
        )
        .unwrap();
        assert!(retained.exported);
        assert_eq!(std::fs::read(retained_output).unwrap(), original);
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
        assert_eq!(
            state
                .sync_engine
                .as_ref()
                .unwrap()
                .status()
                .unwrap()
                .outbox_operations,
            0
        );
        assert!(service.lease_generated_image(&receipt_id).is_ok());
        service.release_generated_image(&receipt_id);
    }

    #[tokio::test]
    async fn export_rejects_mismatched_extension_and_mobile_share_is_explicitly_gated() {
        let (mut state, _vault, _data) = image_test_state();
        let receipt_id = install_receipt(&mut state, "Extension-bound receipt prompt");
        let export_dir = tempfile::tempdir().unwrap();
        let output = export_dir.path().join("misleading.jpg");
        let service = state.openrouter.read().await.clone();

        let error = export_generated_image_to_path_for_test(
            &service,
            ExportGeneratedImageRequest {
                receipt_id,
                retention_policy: ImageMetadataRetentionPolicy::StripMetadata,
            },
            &output,
        )
        .unwrap_err();

        assert!(error.contains("extension"));
        assert!(!output.exists());
        assert!(MOBILE_IMAGE_SHARE_UNAVAILABLE.contains("unavailable"));
    }
}

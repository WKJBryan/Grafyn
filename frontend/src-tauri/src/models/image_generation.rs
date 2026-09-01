use crate::models::note::Note;
use crate::models::twin_event::{ContentDigest, EventId};
use serde::{Deserialize, Serialize};

pub const MAX_IMAGE_RESPONSE_BYTES: usize = 40 * 1024 * 1024;
pub const MAX_GENERATED_IMAGE_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_GENERATED_IMAGE_DIMENSION: u32 = 4096;
pub const MAX_GENERATED_IMAGE_PIXELS: u64 = 16_777_216;
pub const MAX_IMAGE_PROMPT_BYTES: usize = 16 * 1024;
pub const MAX_IMAGE_ANNOTATION_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoverImageModelsRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageModelCapabilityRequest {
    pub model_id: String,
}

impl ImageModelCapabilityRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_model_id(&self.model_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerateImageRequest {
    pub prompt: String,
    pub model_id: String,
    pub resolution: String,
    pub aspect_ratio: String,
}

impl GenerateImageRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.trim().is_empty() {
            return Err("Image prompt cannot be blank".into());
        }
        if self.prompt.len() > MAX_IMAGE_PROMPT_BYTES
            || self
                .prompt
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\r' | '\n' | '\t'))
        {
            return Err("Image prompt must be at most 16 KiB without unsafe controls".into());
        }
        validate_model_id(&self.model_id)?;
        validate_capability_value(&self.resolution, "resolution")?;
        validate_capability_value(&self.aspect_ratio, "aspect ratio")?;
        if self.aspect_ratio != "1:1" {
            return Err("Grafyn currently supports square image generation only".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GeneratedImageSyncPolicy {
    Inherit,
    LocalOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveGeneratedImageRequest {
    pub receipt_id: String,
    #[serde(default)]
    pub annotation: Option<String>,
    #[serde(default)]
    pub retention_policy: ImageMetadataRetentionPolicy,
    pub grafyn_sync: GeneratedImageSyncPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportGeneratedImageRequest {
    pub receipt_id: String,
    #[serde(default)]
    pub retention_policy: ImageMetadataRetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscardGeneratedImageReceiptRequest {
    pub receipt_id: String,
}

impl DiscardGeneratedImageReceiptRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_receipt_id(&self.receipt_id)
    }
}

impl ExportGeneratedImageRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_receipt_id(&self.receipt_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadGeneratedImageRequest {
    pub attachment_digest: ContentDigest,
}

impl SaveGeneratedImageRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_receipt_id(&self.receipt_id)?;
        if let Some(annotation) = self.annotation.as_deref() {
            if annotation.len() > MAX_IMAGE_ANNOTATION_BYTES
                || annotation.chars().any(char::is_control)
            {
                return Err(
                    "Generated image annotation must be at most 2 KiB without controls".into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageMetadataRetentionPolicy {
    #[default]
    StripMetadata,
    RetainOriginal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageGenerationModel {
    pub model_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub resolutions: Vec<String>,
    pub aspect_ratios: Vec<String>,
    pub endpoint_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageEndpointCapability {
    pub endpoint_id: String,
    pub provider: String,
    pub resolutions: Vec<String>,
    pub aspect_ratios: Vec<String>,
    pub output_formats: Vec<String>,
    #[serde(default)]
    pub published_price: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageModelCapabilityResponse {
    pub model_id: String,
    pub endpoints: Vec<ImageEndpointCapability>,
    pub pricing_is_final: bool,
    pub prompt_leaves_device: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageGenerationCost {
    ExactUsd { usd: String },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedImagePreview {
    pub receipt_id: String,
    pub media_type: String,
    pub base64_data: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub model_id: String,
    pub resolution: String,
    pub aspect_ratio: String,
    pub cost: ImageGenerationCost,
    pub prompt_leaves_device: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedGeneratedImage {
    pub note: Note,
    pub observation_event_id: EventId,
    pub attachment_digest: ContentDigest,
    pub media_type: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub sync_disposition: GeneratedImageSyncDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedImageSyncStatus {
    LocalOnly,
    AwaitingProvisioning,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImageSyncDisposition {
    pub status: GeneratedImageSyncStatus,
    pub manifest_count: u32,
    pub chunk_count: u32,
    pub operation_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedGeneratedImage {
    pub attachment_digest: ContentDigest,
    pub media_type: String,
    pub base64_data: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImageExportResult {
    pub exported: bool,
}

fn validate_receipt_id(receipt_id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(receipt_id)
        .map(|_| ())
        .map_err(|_| "Generated image receipt ID is invalid".to_string())
}

fn validate_model_id(model_id: &str) -> Result<(), String> {
    if model_id.len() > 256 || model_id.chars().any(char::is_control) {
        return Err("Image model ID is invalid".into());
    }
    let mut parts = model_id.split('/');
    let Some(author) = parts.next() else {
        return Err("Image model ID must be author/model".into());
    };
    let Some(model) = parts.next() else {
        return Err("Image model ID must be author/model".into());
    };
    if parts.next().is_some()
        || !safe_model_segment(author)
        || !safe_model_segment(model)
        || matches!(author, "." | "..")
        || matches!(model, "." | "..")
    {
        return Err("Image model ID must be a safe author/model identifier".into());
    }
    Ok(())
}

fn safe_model_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_capability_value(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > 64
        || value.chars().any(char::is_control)
        || value.contains(['/', '\\'])
    {
        return Err(format!("Image {label} is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_command_requests_are_strict_and_require_an_explicit_safe_model() {
        let generate = serde_json::json!({
            "prompt": "A quiet future workspace",
            "modelId": "author/model",
            "resolution": "1024x1024",
            "aspectRatio": "1:1"
        });
        assert!(serde_json::from_value::<GenerateImageRequest>(generate.clone()).is_ok());

        let mut unknown = generate.clone();
        unknown["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<GenerateImageRequest>(unknown).is_err());

        for model_id in [
            "",
            "model",
            "../model",
            "author/../model",
            "author/model/extra",
        ] {
            let mut malformed = generate.clone();
            malformed["modelId"] = serde_json::json!(model_id);
            let request = serde_json::from_value::<GenerateImageRequest>(malformed).unwrap();
            assert!(
                request.validate().is_err(),
                "model ID {model_id:?} must fail"
            );
        }

        let blank = GenerateImageRequest {
            prompt: " \n ".into(),
            model_id: "author/model".into(),
            resolution: "1024x1024".into(),
            aspect_ratio: "1:1".into(),
        };
        assert!(blank.validate().is_err());
    }

    #[test]
    fn image_prompt_accepts_multiline_text_exactly_but_rejects_unsafe_controls() {
        let prompt = "First line\r\nSecond line\twith detail\nThird line";
        let request = GenerateImageRequest {
            prompt: prompt.into(),
            model_id: "author/model".into(),
            resolution: "1024x1024".into(),
            aspect_ratio: "1:1".into(),
        };

        assert!(request.validate().is_ok());
        assert_eq!(request.prompt, prompt);
        for control in ['\0', '\u{0008}', '\u{000b}', '\u{007f}'] {
            let mut invalid = request.clone();
            invalid.prompt.push(control);
            assert!(invalid.validate().is_err(), "control {control:?} must fail");
        }
    }

    #[test]
    fn discovery_capability_and_save_requests_deny_unknown_fields() {
        assert!(
            serde_json::from_value::<DiscoverImageModelsRequest>(serde_json::json!({})).is_ok()
        );
        assert!(
            serde_json::from_value::<DiscoverImageModelsRequest>(serde_json::json!({
                "extra": true
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ImageModelCapabilityRequest>(serde_json::json!({
                "modelId": "author/model"
            }))
            .unwrap()
            .validate()
            .is_ok()
        );
        assert!(
            serde_json::from_value::<ImageModelCapabilityRequest>(serde_json::json!({
                "modelId": "author/model",
                "extra": true
            }))
            .is_err()
        );

        let save = serde_json::json!({
            "receiptId": "018f0ca8-2e42-7c1e-ae13-7b35f09b4501",
            "annotation": "Concept sketch",
            "grafynSync": "local_only"
        });
        assert!(
            serde_json::from_value::<SaveGeneratedImageRequest>(save.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        assert_eq!(
            serde_json::from_value::<SaveGeneratedImageRequest>(save.clone())
                .unwrap()
                .retention_policy,
            ImageMetadataRetentionPolicy::StripMetadata
        );
        let mut unknown = save.clone();
        unknown["path"] = serde_json::json!("C:/outside-vault/image.png");
        assert!(serde_json::from_value::<SaveGeneratedImageRequest>(unknown).is_err());
        let mut substituted_prompt = save;
        substituted_prompt["prompt"] = serde_json::json!("frontend replacement prompt");
        assert!(serde_json::from_value::<SaveGeneratedImageRequest>(substituted_prompt).is_err());

        let export = serde_json::json!({
            "receiptId": "018f0ca8-2e42-7c1e-ae13-7b35f09b4501",
            "retentionPolicy": "retain_original"
        });
        let export_request =
            serde_json::from_value::<ExportGeneratedImageRequest>(export.clone()).unwrap();
        assert!(export_request.validate().is_ok());
        assert_eq!(
            export_request.retention_policy,
            ImageMetadataRetentionPolicy::RetainOriginal
        );
        let mut path_injection = export;
        path_injection["path"] = serde_json::json!("/arbitrary/mobile/path.png");
        assert!(serde_json::from_value::<ExportGeneratedImageRequest>(path_injection).is_err());

        let discard = serde_json::json!({
            "receiptId": "018f0ca8-2e42-7c1e-ae13-7b35f09b4501"
        });
        assert!(
            serde_json::from_value::<DiscardGeneratedImageReceiptRequest>(discard.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        let mut discard_unknown = discard;
        discard_unknown["prompt"] = serde_json::json!("must not be accepted");
        assert!(
            serde_json::from_value::<DiscardGeneratedImageReceiptRequest>(discard_unknown).is_err()
        );
    }

    #[test]
    fn generated_preview_cost_is_exact_or_explicitly_unavailable() {
        assert_eq!(
            serde_json::to_value(ImageGenerationCost::ExactUsd {
                usd: "0.004200".into()
            })
            .unwrap(),
            serde_json::json!({"status":"exact_usd","usd":"0.004200"})
        );
        assert_eq!(
            serde_json::to_value(ImageGenerationCost::Unavailable).unwrap(),
            serde_json::json!({"status":"unavailable"})
        );
    }
}

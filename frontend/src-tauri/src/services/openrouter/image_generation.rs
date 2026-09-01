use super::OpenRouterService;
use crate::models::image_generation::{
    GenerateImageRequest, GeneratedImagePreview, ImageEndpointCapability, ImageGenerationCost,
    ImageGenerationModel, ImageModelCapabilityResponse, MAX_GENERATED_IMAGE_BYTES,
    MAX_IMAGE_RESPONSE_BYTES,
};
use crate::models::twin_event::ContentDigest;
use crate::services::attachment_store::validate_generated_image_bytes;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const MAX_IMAGE_DISCOVERY_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_ERROR_BYTES: usize = 64 * 1024;
const MAX_IMAGE_CAPABILITY_EXTENSIONS: usize = 64;
const MAX_IMAGE_CAPABILITY_EXTENSION_BYTES: usize = 256 * 1024;
const MAX_IMAGE_CAPABILITY_EXTENSION_DEPTH: usize = 8;
const MAX_IMAGE_CAPABILITY_EXTENSION_NODES: usize = 4_096;
const MAX_IMAGE_RECEIPTS: usize = 4;
const MAX_IMAGE_RECEIPT_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_RECEIPT_TTL: Duration = Duration::from_secs(15 * 60);

// Dedicated image API contract: https://openrouter.ai/docs/guides/overview/multimodal/image-generation

impl OpenRouterService {
    pub async fn discover_image_models(&self) -> Result<Vec<ImageGenerationModel>> {
        self.require_image_api_key()?;
        let catalog = self.fetch_image_catalog().await?;
        validate_image_catalog(&catalog)?;
        Ok(catalog
            .data
            .into_iter()
            .filter(|model| {
                model
                    .architecture
                    .input_modalities
                    .iter()
                    .any(|modality| modality == "text")
                    && model
                        .architecture
                        .output_modalities
                        .iter()
                        .any(|modality| modality == "image")
            })
            .map(|model| ImageGenerationModel {
                model_id: model.id,
                name: model.name,
                description: model.description,
                resolutions: descriptor_values(model.supported_parameters.resolution.as_ref()),
                aspect_ratios: descriptor_values(model.supported_parameters.aspect_ratio.as_ref()),
                endpoint_path: model.endpoints,
            })
            .collect())
    }

    pub async fn image_model_capability(
        &self,
        model_id: &str,
    ) -> Result<ImageModelCapabilityResponse> {
        self.require_image_api_key()?;
        crate::models::image_generation::ImageModelCapabilityRequest {
            model_id: model_id.to_string(),
        }
        .validate()
        .map_err(|error| anyhow!(error))?;
        let endpoints = self.fetch_image_endpoints(model_id).await?;
        validate_image_endpoints(&endpoints, model_id)?;
        Ok(ImageModelCapabilityResponse {
            model_id: endpoints.id,
            endpoints: endpoints
                .endpoints
                .into_iter()
                .filter_map(|endpoint| {
                    let provider_tag = endpoint.provider_tag.clone()?;
                    Some((provider_tag, endpoint))
                })
                .map(|(provider_tag, endpoint)| ImageEndpointCapability {
                    endpoint_id: provider_tag,
                    provider: endpoint.provider_name,
                    resolutions: descriptor_values(
                        endpoint.supported_parameters.resolution.as_ref(),
                    ),
                    aspect_ratios: descriptor_values(
                        endpoint.supported_parameters.aspect_ratio.as_ref(),
                    ),
                    output_formats: descriptor_values(
                        endpoint.supported_parameters.output_format.as_ref(),
                    ),
                    published_price: unambiguous_output_image_price(&endpoint.pricing),
                })
                .collect(),
            pricing_is_final: false,
            prompt_leaves_device: true,
        })
    }

    pub async fn generate_image(
        &self,
        request: GenerateImageRequest,
        vault_scope: ContentDigest,
    ) -> Result<GeneratedImagePreview> {
        self.require_image_api_key()?;
        request.validate().map_err(|error| anyhow!(error))?;

        let catalog = self.fetch_image_catalog().await?;
        validate_image_catalog(&catalog)?;
        let catalog_model = catalog
            .data
            .iter()
            .find(|model| model.id == request.model_id)
            .ok_or_else(|| {
                anyhow!(
                    "OpenRouter does not currently advertise image model {}",
                    request.model_id
                )
            })?;
        if !catalog_model
            .architecture
            .input_modalities
            .iter()
            .any(|modality| modality == "text")
        {
            bail!(
                "OpenRouter model {} does not advertise text input",
                request.model_id
            );
        }
        if !catalog_model
            .architecture
            .output_modalities
            .iter()
            .any(|modality| modality == "image")
        {
            bail!(
                "OpenRouter model {} does not advertise image output",
                request.model_id
            );
        }

        let endpoints = self.fetch_image_endpoints(&request.model_id).await?;
        validate_image_endpoints(&endpoints, &request.model_id)?;
        let selected = select_image_endpoint(&endpoints.endpoints, &request)?;
        let provider_tag = selected
            .endpoint
            .provider_tag
            .as_deref()
            .expect("selected endpoints are pinnable");
        let pinned_provider = provider_tag.to_owned();
        let api_request = ImageGenerationApiRequest {
            model: &request.model_id,
            prompt: &request.prompt,
            resolution: &request.resolution,
            aspect_ratio: &request.aspect_ratio,
            n: selected.send_n.then_some(1),
            stream: false,
            output_format: selected.output_format.as_deref(),
            provider: ImageGenerationProviderRequest {
                only: [provider_tag],
                allow_fallbacks: false,
            },
        };
        let response = self
            .client
            .post(format!(
                "{}/images",
                self.image_api_url.trim_end_matches('/')
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://grafyn.app")
            .header("X-Title", "Grafyn")
            .json(&api_request)
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .context("Failed to send OpenRouter image request")?;
        let response = read_image_api_response(response, MAX_IMAGE_RESPONSE_BYTES).await?;
        let parsed: ImageGenerationApiResponse = serde_json::from_slice(&response)
            .context("OpenRouter image response did not match the expected schema")?;
        let _ = parsed.created;
        validate_image_generation_usage(parsed.usage.as_ref())?;
        if parsed.data.len() != 1 {
            bail!("OpenRouter image response must contain exactly one image");
        }
        let item = parsed.data.into_iter().next().expect("length checked");
        let encoded_limit = MAX_GENERATED_IMAGE_BYTES.div_ceil(3) * 4;
        if item.b64_json.is_empty() || item.b64_json.len() > encoded_limit {
            bail!("OpenRouter image payload is empty or exceeds 24 MiB");
        }
        let image_bytes = STANDARD
            .decode(item.b64_json.as_bytes())
            .context("OpenRouter image payload is not canonical standard base64")?;
        if STANDARD.encode(&image_bytes) != item.b64_json {
            bail!("OpenRouter image payload is not canonical standard base64");
        }
        let validated = validate_generated_image_bytes(&image_bytes, item.media_type.as_deref())
            .map_err(|error| anyhow!(error))?;
        let receipt = GeneratedImageReceipt {
            bytes: image_bytes,
            media_type: validated.media_type().to_string(),
            width: validated.dimensions().0,
            height: validated.dimensions().1,
            prompt: request.prompt.clone(),
            model_id: request.model_id.clone(),
            resolution: request.resolution.clone(),
            aspect_ratio: request.aspect_ratio.clone(),
            vault_scope,
            gateway: "openrouter".into(),
            provider_tag: pinned_provider,
        };
        let receipt_id = self
            .image_receipts
            .lock()
            .map_err(|_| anyhow!("Generated image receipt store is unavailable"))?
            .insert(receipt.clone())?;
        self.schedule_generated_image_expiry(IMAGE_RECEIPT_TTL);
        Ok(GeneratedImagePreview {
            receipt_id,
            media_type: receipt.media_type,
            base64_data: item.b64_json,
            byte_size: receipt.bytes.len() as u64,
            width: receipt.width,
            height: receipt.height,
            model_id: receipt.model_id,
            resolution: receipt.resolution,
            aspect_ratio: receipt.aspect_ratio,
            cost: parsed
                .usage
                .and_then(|usage| usage.cost)
                .map(|cost| ImageGenerationCost::ExactUsd {
                    usd: cost.to_string(),
                })
                .unwrap_or(ImageGenerationCost::Unavailable),
            prompt_leaves_device: true,
        })
    }

    pub(crate) fn lease_generated_image(&self, receipt_id: &str) -> Result<GeneratedImageReceipt> {
        self.image_receipts
            .lock()
            .map_err(|_| anyhow!("Generated image receipt store is unavailable"))?
            .lease(receipt_id)
    }

    pub(crate) fn release_generated_image(&self, receipt_id: &str) {
        if let Ok(mut receipts) = self.image_receipts.lock() {
            receipts.release(receipt_id);
        }
    }

    pub(crate) fn consume_generated_image(&self, receipt_id: &str) {
        if let Ok(mut receipts) = self.image_receipts.lock() {
            receipts.consume(receipt_id);
        }
    }

    fn schedule_generated_image_expiry(&self, delay: Duration) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let receipts = Arc::downgrade(&self.image_receipts);
        runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some(receipts) = receipts.upgrade() {
                if let Ok(mut receipts) = receipts.lock() {
                    receipts.purge_expired();
                }
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn insert_generated_image_for_tests(
        &self,
        receipt: GeneratedImageReceipt,
    ) -> Result<String> {
        let receipt_id = self
            .image_receipts
            .lock()
            .map_err(|_| anyhow!("Generated image receipt store is unavailable"))?
            .insert(receipt)?;
        self.schedule_generated_image_expiry(IMAGE_RECEIPT_TTL);
        Ok(receipt_id)
    }

    #[cfg(test)]
    pub(crate) fn insert_generated_image_for_tests_with_ttl(
        &self,
        receipt: GeneratedImageReceipt,
        ttl: Duration,
    ) -> Result<String> {
        let receipt_id = self
            .image_receipts
            .lock()
            .map_err(|_| anyhow!("Generated image receipt store is unavailable"))?
            .insert_with_ttl(receipt, ttl)?;
        self.schedule_generated_image_expiry(ttl);
        Ok(receipt_id)
    }

    fn require_image_api_key(&self) -> Result<()> {
        if !self.is_configured() {
            bail!("OpenRouter API key not configured");
        }
        Ok(())
    }

    async fn fetch_image_catalog(&self) -> Result<ImageCatalogResponse> {
        let bytes = self
            .send_image_get("/images/models", MAX_IMAGE_DISCOVERY_BYTES)
            .await?;
        serde_json::from_slice(&bytes)
            .context("OpenRouter image model discovery did not match the expected schema")
    }

    async fn fetch_image_endpoints(&self, model_id: &str) -> Result<ImageEndpointsResponse> {
        let bytes = self
            .send_image_get(
                &format!("/images/models/{model_id}/endpoints"),
                MAX_IMAGE_DISCOVERY_BYTES,
            )
            .await?;
        serde_json::from_slice(&bytes)
            .context("OpenRouter image endpoint discovery did not match the expected schema")
    }

    async fn send_image_get(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(format!(
                "{}{path}",
                self.image_api_url.trim_end_matches('/')
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("Failed to query OpenRouter image capabilities")?;
        read_image_api_response(response, limit).await
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageCatalogResponse {
    data: Vec<ImageCatalogModel>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageCatalogModel {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    created: u64,
    architecture: ImageModelArchitecture,
    supported_parameters: ImageSupportedParameters,
    supports_streaming: bool,
    endpoints: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageModelArchitecture {
    input_modalities: Vec<String>,
    output_modalities: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ImageSupportedParameters {
    #[serde(default)]
    aspect_ratio: Option<ImageParameterDescriptor>,
    #[serde(default)]
    background: Option<ImageParameterDescriptor>,
    #[serde(default)]
    input_references: Option<ImageParameterDescriptor>,
    #[serde(default)]
    n: Option<ImageParameterDescriptor>,
    #[serde(default)]
    output_compression: Option<ImageParameterDescriptor>,
    #[serde(default)]
    output_format: Option<ImageParameterDescriptor>,
    #[serde(default)]
    quality: Option<ImageParameterDescriptor>,
    #[serde(default)]
    resolution: Option<ImageParameterDescriptor>,
    #[serde(default)]
    seed: Option<ImageParameterDescriptor>,
    #[serde(default)]
    size: Option<ImageParameterDescriptor>,
    #[serde(flatten)]
    additional: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ImageParameterDescriptor {
    Enum {
        values: Vec<String>,
        #[serde(flatten)]
        extensions: HashMap<String, serde_json::Value>,
    },
    Range {
        min: serde_json::Number,
        max: serde_json::Number,
        #[serde(flatten)]
        extensions: HashMap<String, serde_json::Value>,
    },
    Boolean {
        #[serde(flatten)]
        extensions: HashMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageEndpointsResponse {
    id: String,
    endpoints: Vec<ImageEndpointDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageEndpointDescriptor {
    provider_name: String,
    provider_slug: String,
    provider_tag: Option<String>,
    supported_parameters: ImageSupportedParameters,
    allowed_passthrough_parameters: Vec<String>,
    supports_streaming: bool,
    pricing: Vec<ImageEndpointPricing>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageEndpointPricing {
    billable: String,
    unit: String,
    cost_usd: serde_json::Number,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Debug, Serialize)]
struct ImageGenerationApiRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    resolution: &'a str,
    aspect_ratio: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u8>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_format: Option<&'a str>,
    provider: ImageGenerationProviderRequest<'a>,
}

#[derive(Debug, Serialize)]
struct ImageGenerationProviderRequest<'a> {
    only: [&'a str; 1],
    allow_fallbacks: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageGenerationApiResponse {
    created: u64,
    data: Vec<ImageGenerationApiItem>,
    #[serde(default)]
    usage: Option<ImageGenerationUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageGenerationApiItem {
    b64_json: String,
    #[serde(default)]
    media_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageGenerationUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    cost: Option<serde_json::Number>,
}

#[derive(Debug, Clone)]
pub(crate) struct GeneratedImageReceipt {
    pub(crate) bytes: Vec<u8>,
    pub(crate) media_type: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) prompt: String,
    pub(crate) model_id: String,
    pub(crate) resolution: String,
    pub(crate) aspect_ratio: String,
    pub(crate) vault_scope: ContentDigest,
    pub(crate) gateway: String,
    pub(crate) provider_tag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptState {
    Ready,
    Leased,
}

#[derive(Debug)]
struct StoredImageReceipt {
    receipt: GeneratedImageReceipt,
    expires_at: Instant,
    state: ReceiptState,
}

#[derive(Debug, Default)]
pub(super) struct ImageReceiptStore {
    entries: HashMap<String, StoredImageReceipt>,
    order: VecDeque<String>,
    total_bytes: usize,
}

impl ImageReceiptStore {
    fn insert(&mut self, receipt: GeneratedImageReceipt) -> Result<String> {
        self.insert_with_ttl(receipt, IMAGE_RECEIPT_TTL)
    }

    fn insert_with_ttl(&mut self, receipt: GeneratedImageReceipt, ttl: Duration) -> Result<String> {
        self.purge_expired();
        if receipt.bytes.is_empty() || receipt.bytes.len() > MAX_GENERATED_IMAGE_BYTES {
            bail!("Generated image receipt exceeds the allowed image size");
        }
        while self.entries.len() >= MAX_IMAGE_RECEIPTS
            || self.total_bytes + receipt.bytes.len() > MAX_IMAGE_RECEIPT_BYTES
        {
            let Some(position) = self.order.iter().position(|id| {
                self.entries
                    .get(id)
                    .is_some_and(|entry| entry.state == ReceiptState::Ready)
            }) else {
                bail!("Generated image receipt store is busy");
            };
            let evicted = self.order.remove(position).expect("position checked");
            self.remove(&evicted);
        }
        let receipt_id = uuid::Uuid::new_v4().to_string();
        self.total_bytes += receipt.bytes.len();
        self.order.push_back(receipt_id.clone());
        self.entries.insert(
            receipt_id.clone(),
            StoredImageReceipt {
                receipt,
                expires_at: Instant::now() + ttl,
                state: ReceiptState::Ready,
            },
        );
        Ok(receipt_id)
    }

    fn lease(&mut self, receipt_id: &str) -> Result<GeneratedImageReceipt> {
        self.purge_expired();
        let entry = self
            .entries
            .get_mut(receipt_id)
            .ok_or_else(|| anyhow!("Generated image receipt is missing or expired"))?;
        if entry.state != ReceiptState::Ready {
            bail!("Generated image receipt is already being saved");
        }
        entry.state = ReceiptState::Leased;
        Ok(entry.receipt.clone())
    }

    fn release(&mut self, receipt_id: &str) {
        self.purge_expired();
        if let Some(entry) = self.entries.get_mut(receipt_id) {
            if entry.state == ReceiptState::Leased {
                entry.state = ReceiptState::Ready;
            }
        }
    }

    fn consume(&mut self, receipt_id: &str) {
        self.remove(receipt_id);
        self.order.retain(|id| id != receipt_id);
    }

    fn purge_expired(&mut self) {
        let now = Instant::now();
        let expired = self
            .entries
            .iter()
            .filter_map(|(id, entry)| (entry.expires_at <= now).then(|| id.clone()))
            .collect::<Vec<_>>();
        for id in expired {
            self.remove(&id);
        }
        self.order.retain(|id| self.entries.contains_key(id));
    }

    fn remove(&mut self, receipt_id: &str) {
        if let Some(removed) = self.entries.remove(receipt_id) {
            self.total_bytes = self.total_bytes.saturating_sub(removed.receipt.bytes.len());
        }
    }
}

struct SelectedImageEndpoint<'a> {
    endpoint: &'a ImageEndpointDescriptor,
    output_format: Option<String>,
    send_n: bool,
}

fn select_image_endpoint<'a>(
    endpoints: &'a [ImageEndpointDescriptor],
    request: &GenerateImageRequest,
) -> Result<SelectedImageEndpoint<'a>> {
    let mut candidates = endpoints
        .iter()
        .filter_map(|endpoint| {
            if endpoint.provider_tag.is_none()
                || !descriptor_contains(
                    endpoint.supported_parameters.resolution.as_ref(),
                    &request.resolution,
                )
                || !descriptor_contains(
                    endpoint.supported_parameters.aspect_ratio.as_ref(),
                    &request.aspect_ratio,
                )
                || endpoint
                    .supported_parameters
                    .n
                    .as_ref()
                    .is_some_and(|descriptor| !descriptor_range_contains(Some(descriptor), 1))
                || endpoint
                    .supported_parameters
                    .input_references
                    .as_ref()
                    .is_some_and(|descriptor| !descriptor_range_contains(Some(descriptor), 0))
            {
                return None;
            }
            let output_format = match endpoint.supported_parameters.output_format.as_ref() {
                None => None,
                Some(descriptor) => {
                    let output_formats = descriptor_values(Some(descriptor));
                    Some(
                        ["png", "jpeg", "jpg", "webp"]
                            .iter()
                            .find_map(|preferred| {
                                output_formats
                                    .iter()
                                    .find(|format| format.eq_ignore_ascii_case(preferred))
                                    .cloned()
                            })?,
                    )
                }
            };
            Some(SelectedImageEndpoint {
                endpoint,
                output_format,
                send_n: endpoint.supported_parameters.n.is_some(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.endpoint.provider_tag.cmp(&right.endpoint.provider_tag));
    candidates.into_iter().next().ok_or_else(|| {
        anyhow!(
            "OpenRouter image endpoint does not advertise the requested resolution, aspect ratio, count, prompt-only input references, and raster format"
        )
    })
}

fn descriptor_values(descriptor: Option<&ImageParameterDescriptor>) -> Vec<String> {
    match descriptor {
        Some(ImageParameterDescriptor::Enum { values, .. }) => values.clone(),
        _ => Vec::new(),
    }
}

fn descriptor_contains(descriptor: Option<&ImageParameterDescriptor>, expected: &str) -> bool {
    matches!(
        descriptor,
        Some(ImageParameterDescriptor::Enum { values, .. }) if values.iter().any(|value| value == expected)
    )
}

fn descriptor_range_contains(descriptor: Option<&ImageParameterDescriptor>, expected: i64) -> bool {
    matches!(
        descriptor,
        Some(ImageParameterDescriptor::Range { min, max, .. })
            if min.as_i64().is_some_and(|min| min <= expected)
                && max.as_i64().is_some_and(|max| max >= expected)
    )
}

fn validate_image_catalog(catalog: &ImageCatalogResponse) -> Result<()> {
    if catalog.data.len() > 256 {
        bail!("OpenRouter image discovery returned too many models");
    }
    let mut model_ids = HashSet::new();
    for model in &catalog.data {
        crate::models::image_generation::ImageModelCapabilityRequest {
            model_id: model.id.clone(),
        }
        .validate()
        .map_err(|error| anyhow!(error))?;
        if !model_ids.insert(model.id.as_str()) {
            bail!("OpenRouter image discovery contains a duplicate model ID");
        }
        if model.name.is_empty()
            || model.name.len() > 512
            || model.name.chars().any(char::is_control)
            || model.description.as_ref().is_some_and(|value| {
                value.len() > 16 * 1024 || value.chars().any(is_unsafe_text_control)
            })
            || model.architecture.input_modalities.len() > 32
            || model.architecture.output_modalities.len() > 32
            || model.endpoints != format!("/api/v1/images/models/{}/endpoints", model.id)
        {
            bail!("OpenRouter image discovery contains invalid bounded metadata");
        }
        validate_bounded_identifiers(
            &model.architecture.input_modalities,
            32,
            128,
            "input modalities",
        )?;
        validate_bounded_identifiers(
            &model.architecture.output_modalities,
            32,
            128,
            "output modalities",
        )?;
        let _ = model.created;
        let _ = model.supports_streaming;
        validate_supported_parameters(&model.supported_parameters)?;
    }
    Ok(())
}

fn validate_image_endpoints(
    endpoints: &ImageEndpointsResponse,
    expected_model: &str,
) -> Result<()> {
    if endpoints.id != expected_model || endpoints.endpoints.len() > 64 {
        bail!("OpenRouter image endpoint response does not match the requested model");
    }
    let mut provider_tags = HashSet::new();
    for endpoint in &endpoints.endpoints {
        if endpoint.provider_name.is_empty()
            || endpoint.provider_name.len() > 512
            || endpoint.provider_name.chars().any(char::is_control)
            || endpoint.provider_slug.is_empty()
            || endpoint.provider_slug.len() > 256
            || endpoint.provider_slug.chars().any(char::is_control)
            || endpoint.allowed_passthrough_parameters.len() > 64
            || endpoint.pricing.len() > 64
        {
            bail!("OpenRouter image endpoint contains invalid bounded metadata");
        }
        validate_bounded_identifiers(
            &endpoint.allowed_passthrough_parameters,
            64,
            128,
            "passthrough parameters",
        )?;
        if let Some(provider_tag) = endpoint.provider_tag.as_deref() {
            if provider_tag.is_empty()
                || provider_tag.len() > 256
                || provider_tag.chars().any(char::is_control)
            {
                bail!("OpenRouter image endpoint contains invalid provider tag");
            }
            if !provider_tags.insert(provider_tag) {
                bail!("OpenRouter image endpoint response contains a duplicate provider tag");
            }
        }
        validate_supported_parameters(&endpoint.supported_parameters)?;
        for price in &endpoint.pricing {
            if price.billable.is_empty()
                || price.billable.len() > 128
                || price.billable.chars().any(char::is_control)
                || price.unit.is_empty()
                || price.unit.len() > 128
                || price.unit.chars().any(char::is_control)
                || price.variant.as_ref().is_some_and(|value| {
                    value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
                })
            {
                bail!("OpenRouter image endpoint contains invalid pricing metadata");
            }
            validate_nonnegative_bounded_decimal(&price.cost_usd, "published price")?;
        }
        let _ = endpoint.supports_streaming;
    }
    Ok(())
}

fn validate_bounded_identifiers(
    values: &[String],
    max_count: usize,
    max_len: usize,
    label: &str,
) -> Result<()> {
    if values.len() > max_count
        || values.iter().any(|value| {
            value.is_empty() || value.len() > max_len || value.chars().any(char::is_control)
        })
    {
        bail!("OpenRouter image {label} contains invalid bounded values");
    }
    Ok(())
}

fn validate_image_generation_usage(usage: Option<&ImageGenerationUsage>) -> Result<()> {
    if usage.is_some_and(|usage| {
        [
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens,
        ]
        .into_iter()
        .flatten()
        .any(|tokens| tokens > 1_000_000_000)
    }) {
        bail!("OpenRouter image response contains invalid usage metadata");
    }
    if let Some(cost) = usage.and_then(|usage| usage.cost.as_ref()) {
        validate_nonnegative_bounded_decimal(cost, "usage cost")?;
    }
    Ok(())
}

fn unambiguous_output_image_price(prices: &[ImageEndpointPricing]) -> Option<String> {
    let mut matches = prices.iter().filter(|price| {
        price.billable == "output_image" && price.unit == "image" && price.variant.is_none()
    });
    let price = matches.next()?;
    matches.next().is_none().then(|| price.cost_usd.to_string())
}

fn validate_nonnegative_bounded_decimal(value: &serde_json::Number, label: &str) -> Result<()> {
    let text = value.to_string();
    if text.is_empty() || text.len() > 64 || text.starts_with('-') {
        bail!("OpenRouter image {label} must be a bounded non-negative decimal");
    }
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text.as_str(), 0), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap_or(i32::MAX))
        });
    let integer_digits = mantissa
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_start_matches('0')
        .len() as i32;
    if exponent < -32 || exponent > 12 || integer_digits.saturating_add(exponent) > 12 {
        bail!("OpenRouter image {label} must be a bounded non-negative decimal");
    }
    Ok(())
}

fn is_unsafe_text_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
}

fn validate_supported_parameters(parameters: &ImageSupportedParameters) -> Result<()> {
    validate_capability_extensions(&parameters.additional)?;
    let known = [
        parameters.aspect_ratio.as_ref(),
        parameters.background.as_ref(),
        parameters.input_references.as_ref(),
        parameters.n.as_ref(),
        parameters.output_compression.as_ref(),
        parameters.output_format.as_ref(),
        parameters.quality.as_ref(),
        parameters.resolution.as_ref(),
        parameters.seed.as_ref(),
        parameters.size.as_ref(),
    ];
    for descriptor in known.into_iter().flatten() {
        let extensions = match descriptor {
            ImageParameterDescriptor::Enum { values, extensions } => {
                if values.len() > 128
                    || values.iter().any(|value| {
                        value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
                    })
                {
                    bail!("OpenRouter image parameter contains invalid bounded values");
                }
                extensions
            }
            ImageParameterDescriptor::Range {
                min,
                max,
                extensions,
            } => {
                if min.to_string().len() > 64 || max.to_string().len() > 64 {
                    bail!("OpenRouter image parameter contains invalid bounded values");
                }
                extensions
            }
            ImageParameterDescriptor::Boolean { extensions } => extensions,
        };
        validate_capability_extensions(extensions)?;
    }
    Ok(())
}

fn validate_capability_extensions(extensions: &HashMap<String, serde_json::Value>) -> Result<()> {
    if extensions.len() > MAX_IMAGE_CAPABILITY_EXTENSIONS
        || extensions
            .keys()
            .any(|name| name.is_empty() || name.len() > 128 || name.chars().any(char::is_control))
    {
        bail!("OpenRouter image parameter extensions contain invalid bounded values");
    }
    let mut total_bytes = 0usize;
    let mut total_nodes = 0usize;
    for value in extensions.values() {
        total_bytes = total_bytes
            .checked_add(serde_json::to_vec(value)?.len())
            .ok_or_else(|| anyhow!("OpenRouter image parameter extensions exceed bounds"))?;
        total_nodes = total_nodes
            .checked_add(validate_capability_extension_value(value, 0)?)
            .ok_or_else(|| anyhow!("OpenRouter image parameter extensions exceed bounds"))?;
        if total_bytes > MAX_IMAGE_CAPABILITY_EXTENSION_BYTES
            || total_nodes > MAX_IMAGE_CAPABILITY_EXTENSION_NODES
        {
            bail!("OpenRouter image parameter extensions exceed bounds");
        }
    }
    Ok(())
}

fn validate_capability_extension_value(value: &serde_json::Value, depth: usize) -> Result<usize> {
    if depth > MAX_IMAGE_CAPABILITY_EXTENSION_DEPTH {
        bail!("OpenRouter image parameter extensions exceed bounds");
    }
    let mut nodes = 1usize;
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) => {}
        serde_json::Value::Number(number) => {
            if number.to_string().len() > 64 {
                bail!("OpenRouter image parameter extensions exceed bounds");
            }
        }
        serde_json::Value::String(value) => {
            if value.len() > 4_096 || value.chars().any(is_unsafe_text_control) {
                bail!("OpenRouter image parameter extensions exceed bounds");
            }
        }
        serde_json::Value::Array(values) => {
            if values.len() > 128 {
                bail!("OpenRouter image parameter extensions exceed bounds");
            }
            for value in values {
                nodes = nodes
                    .checked_add(validate_capability_extension_value(value, depth + 1)?)
                    .ok_or_else(|| {
                        anyhow!("OpenRouter image parameter extensions exceed bounds")
                    })?;
            }
        }
        serde_json::Value::Object(values) => {
            if values.len() > 64
                || values.keys().any(|name| {
                    name.is_empty() || name.len() > 128 || name.chars().any(char::is_control)
                })
            {
                bail!("OpenRouter image parameter extensions exceed bounds");
            }
            for value in values.values() {
                nodes = nodes
                    .checked_add(validate_capability_extension_value(value, depth + 1)?)
                    .ok_or_else(|| {
                        anyhow!("OpenRouter image parameter extensions exceed bounds")
                    })?;
            }
        }
    }
    if nodes > MAX_IMAGE_CAPABILITY_EXTENSION_NODES {
        bail!("OpenRouter image parameter extensions exceed bounds");
    }
    Ok(nodes)
}

async fn read_image_api_response(
    response: reqwest::Response,
    success_limit: usize,
) -> Result<Vec<u8>> {
    let status = response.status();
    let limit = if status.is_success() {
        success_limit
    } else {
        MAX_IMAGE_ERROR_BYTES
    };
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!("OpenRouter image response exceeded the {limit}-byte limit");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed while reading OpenRouter image response")?;
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow!("OpenRouter image response length overflow"))?;
        if next_len > limit {
            bail!("OpenRouter image response exceeded the {limit}-byte limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let parsed = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
        let message = parsed
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .filter(|message| {
                !message.is_empty()
                    && message.len() <= 4096
                    && !message.chars().any(char::is_control)
            })
            .unwrap_or("OpenRouter image request failed");
        let code = parsed
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.get("code"))
            .and_then(|value| match value {
                serde_json::Value::String(value)
                    if !value.is_empty()
                        && value.len() <= 128
                        && !value.chars().any(char::is_control) =>
                {
                    Some(value.clone())
                }
                serde_json::Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| status.as_u16().to_string());
        bail!("OpenRouter image API error {code}: {message}");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;

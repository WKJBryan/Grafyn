use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Listener, Manager};

pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 48 * 1024 * 1024;
const MAX_RESPONSE_EVENTS: usize = 4096;
const CANVAS_TERMINAL_TIMEOUT: Duration = Duration::from_secs(20);
const CANVAS_TERMINAL_TIMEOUT_ERROR: &str = "E2E Canvas stream did not reach a terminal event";
const DEFAULT_RUNTIME_PORT: u16 = 18890;
const DEFAULT_OPENROUTER_URL: &str = "http://127.0.0.1:18891/api/v1";
const OWNED_ROOT_MARKER_NAME: &str = ".grafyn-e2e-owned-v1";
const OWNED_ROOT_MARKER_BYTES: &[u8] = b"grafyn-e2e-owned-root-v1\n";
const CORS_REQUEST_HEADERS: &str =
    "authorization,content-type,x-grafyn-e2e-device,x-grafyn-e2e-profile";

const ALLOWLISTED_COMMANDS: &[&str] = &[
    "get_runtime_status",
    "get_boot_status",
    "get_settings",
    "get_settings_status",
    "update_settings",
    "get_openrouter_status",
    "get_vault_optimizer_status",
    "get_vault_optimizer_inbox",
    "get_mcp_status",
    "get_sync_status",
    "list_notes",
    "get_note",
    "create_note",
    "update_note",
    "delete_note",
    "search_notes",
    "get_full_graph",
    "get_backlinks",
    "list_sessions",
    "get_session",
    "create_session",
    "update_session",
    "delete_session",
    "get_available_models",
    "send_prompt",
    "regenerate_response",
    "get_twin_review",
    "run_twin_inference",
    "record_canvas_feedback",
    "list_decision_episodes",
    "get_decision_mirror_config",
    "list_memory_digest",
    "review_memory_digest_item",
    "list_constitution_items",
    "list_action_gaps",
    "get_constitution_setup",
    "save_constitution_setup",
    "list_link_suggestion_queue",
    "get_link_discovery_status",
    "list_twin_observations",
    "list_twin_proposals",
    "create_companion_capture",
    "review_twin_proposal",
    "get_twin_state_projection",
    "rank_twin_attention",
    "get_twin_event_timeline",
    "discover_image_models",
    "get_image_model_capability",
    "generate_image",
    "discard_generated_image_receipt",
    "save_generated_image",
    "load_generated_image",
    "recall_relevant",
    "find_contradictions",
    "list_sync_conflicts",
    "export_sync_outbox",
    "import_sync_envelopes",
    "rebuild_sync_state",
    "e2e_restart_runtime",
];

const ANDROID_PROFILE_COMMANDS: &[&str] = &[
    "get_runtime_status",
    "get_boot_status",
    "get_settings",
    "get_settings_status",
    "update_settings",
    "get_openrouter_status",
    "get_sync_status",
    "list_notes",
    "get_note",
    "create_note",
    "update_note",
    "delete_note",
    "list_sessions",
    "get_session",
    "create_session",
    "update_session",
    "delete_session",
    "get_available_models",
    "send_prompt",
    "regenerate_response",
    "get_twin_review",
    "record_canvas_feedback",
    "list_decision_episodes",
    "get_decision_mirror_config",
    "list_memory_digest",
    "review_memory_digest_item",
    "list_constitution_items",
    "list_action_gaps",
    "get_constitution_setup",
    "list_twin_observations",
    "list_twin_proposals",
    "create_companion_capture",
    "review_twin_proposal",
    "get_twin_state_projection",
    "rank_twin_attention",
    "get_twin_event_timeline",
    "discover_image_models",
    "get_image_model_capability",
    "generate_image",
    "discard_generated_image_receipt",
    "save_generated_image",
    "load_generated_image",
    "recall_relevant",
];

const HARNESS_ONLY_COMMANDS: &[&str] = &[
    "save_constitution_setup",
    "list_sync_conflicts",
    "export_sync_outbox",
    "import_sync_envelopes",
    "rebuild_sync_state",
    "e2e_restart_runtime",
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TestDevice {
    DeviceA,
    DeviceB,
}

impl TestDevice {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "device-a" => Ok(Self::DeviceA),
            "device-b" => Ok(Self::DeviceB),
            _ => Err("E2E device must be device-a or device-b".to_string()),
        }
    }

    fn root_name(self) -> &'static str {
        match self {
            Self::DeviceA => "device-a",
            Self::DeviceB => "device-b",
        }
    }

    fn writer_device_id(self) -> &'static str {
        match self {
            Self::DeviceA => "123e4567-e89b-42d3-a456-4266141740a1",
            Self::DeviceB => "123e4567-e89b-42d3-a456-4266141740b2",
        }
    }

    fn signing_seed(self) -> [u8; 32] {
        match self {
            Self::DeviceA => [0xa1; 32],
            Self::DeviceB => [0xb2; 32],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TestProfile {
    Android,
    Desktop,
}

impl TestProfile {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "android" => Ok(Self::Android),
            "desktop" => Ok(Self::Desktop),
            _ => Err("E2E profile must be android or desktop".to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TestRuntimeConfigInput {
    pub root: PathBuf,
    pub port: u16,
    pub origin: String,
    pub bearer_token: String,
    pub openrouter_url: String,
}

#[derive(Clone, Debug)]
pub struct TestRuntimeConfig {
    root: PathBuf,
    bind_addr: SocketAddr,
    origin: String,
    bearer_token: String,
    openrouter_url: String,
}

impl TestRuntimeConfig {
    pub fn from_env() -> Result<Self, String> {
        let root = std::env::var_os("GRAFYN_E2E_RUNTIME_ROOT")
            .map(PathBuf::from)
            .ok_or_else(|| "GRAFYN_E2E_RUNTIME_ROOT is required".to_string())?;
        let port = match std::env::var("GRAFYN_E2E_RUNTIME_PORT") {
            Ok(value) => value
                .parse::<u16>()
                .map_err(|_| "GRAFYN_E2E_RUNTIME_PORT must be a non-zero u16".to_string())?,
            Err(std::env::VarError::NotPresent) => DEFAULT_RUNTIME_PORT,
            Err(_) => return Err("GRAFYN_E2E_RUNTIME_PORT is invalid".to_string()),
        };
        let origin = std::env::var("GRAFYN_E2E_RUNTIME_ORIGIN")
            .map_err(|_| "GRAFYN_E2E_RUNTIME_ORIGIN is required".to_string())?;
        let bearer_token = std::env::var("GRAFYN_E2E_RUNTIME_TOKEN")
            .map_err(|_| "GRAFYN_E2E_RUNTIME_TOKEN is required".to_string())?;
        let openrouter_url = std::env::var("GRAFYN_E2E_OPENROUTER_URL")
            .unwrap_or_else(|_| DEFAULT_OPENROUTER_URL.to_string());
        Self::from_values(TestRuntimeConfigInput {
            root,
            port,
            origin,
            bearer_token,
            openrouter_url,
        })
    }

    pub fn from_values(input: TestRuntimeConfigInput) -> Result<Self, String> {
        if input.port == 0 {
            return Err("runtime port must be non-zero".to_string());
        }
        validate_owned_root(&input.root)?;
        let expected_origin = format!("http://127.0.0.1:{}", explicit_port(&input.origin)?);
        if input.origin != expected_origin {
            return Err("runtime Origin must be an exact http://127.0.0.1:<port> authority".into());
        }
        validate_openrouter_url(&input.openrouter_url)?;
        if input.bearer_token.len() != 64
            || !input
                .bearer_token
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("runtime token must be exactly 64 lowercase hexadecimal characters".into());
        }
        Ok(Self {
            root: input.root,
            bind_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, input.port)),
            origin: input.origin,
            bearer_token: input.bearer_token,
            openrouter_url: input.openrouter_url,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }
}

fn validate_owned_root(root: &std::path::Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("runtime root must be absolute".to_string());
    }
    std::fs::create_dir_all(root)
        .map_err(|error| format!("runtime root is unavailable: {error}"))?;
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|error| format!("runtime root is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("runtime root must be a real directory".to_string());
    }
    let marker = root.join(OWNED_ROOT_MARKER_NAME);
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("runtime root ownership marker is invalid".to_string());
            }
            let bytes = std::fs::read(&marker)
                .map_err(|_| "runtime root ownership marker is unreadable".to_string())?;
            if bytes != OWNED_ROOT_MARKER_BYTES {
                return Err("runtime root ownership marker is invalid".to_string());
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::read_dir(root)
                .map_err(|error| format!("runtime root is unavailable: {error}"))?
                .next()
                .is_some()
            {
                return Err(
                    "runtime root must be empty or carry Grafyn's E2E ownership marker".to_string(),
                );
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)
                .map_err(|error| format!("runtime root ownership marker failed: {error}"))?;
            std::io::Write::write_all(&mut file, OWNED_ROOT_MARKER_BYTES)
                .map_err(|error| format!("runtime root ownership marker failed: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("runtime root ownership marker failed: {error}"))?;
            Ok(())
        }
        Err(error) => Err(format!("runtime root ownership marker failed: {error}")),
    }
}

fn explicit_port(authority: &str) -> Result<u16, String> {
    let parsed = reqwest::Url::parse(authority)
        .map_err(|_| "runtime Origin must be an absolute URL".to_string())?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("runtime Origin must be an exact loopback authority".to_string());
    }
    parsed
        .port()
        .ok_or_else(|| "runtime Origin must include an explicit port".to_string())
}

fn validate_openrouter_url(value: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| "OpenRouter stub URL must be an absolute URL".to_string())?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.port().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.path() != "/api/v1"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("OpenRouter stub URL must be an exact loopback /api/v1 URL".to_string());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvokeEnvelope {
    command: String,
    args: Map<String, Value>,
}

fn parse_invoke_envelope(bytes: &[u8]) -> Result<InvokeEnvelope, String> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err("invoke request exceeds 1 MiB".to_string());
    }
    let envelope: InvokeEnvelope =
        serde_json::from_slice(bytes).map_err(|_| "invoke request is invalid".to_string())?;
    if !is_allowlisted_command(&envelope.command) {
        return Err("invoke command is unavailable".to_string());
    }
    Ok(envelope)
}

fn is_allowlisted_command(command: &str) -> bool {
    ALLOWLISTED_COMMANDS.binary_search(&command).is_ok() || ALLOWLISTED_COMMANDS.contains(&command)
}

fn is_command_allowed_for_profile(profile: TestProfile, command: &str) -> bool {
    match profile {
        TestProfile::Desktop => is_allowlisted_command(command),
        TestProfile::Android => {
            ANDROID_PROFILE_COMMANDS.contains(&command) || HARNESS_ONLY_COMMANDS.contains(&command)
        }
    }
}

fn expected_argument_names(command: &str) -> &'static [&'static str] {
    match command {
        "get_runtime_status"
        | "get_boot_status"
        | "get_settings"
        | "get_settings_status"
        | "get_openrouter_status"
        | "get_sync_status"
        | "list_notes"
        | "get_full_graph"
        | "get_link_discovery_status"
        | "list_sessions"
        | "get_available_models"
        | "get_twin_review"
        | "run_twin_inference"
        | "list_decision_episodes"
        | "get_decision_mirror_config"
        | "list_memory_digest"
        | "list_constitution_items"
        | "list_action_gaps"
        | "get_constitution_setup"
        | "list_sync_conflicts"
        | "export_sync_outbox"
        | "rebuild_sync_state"
        | "e2e_restart_runtime" => &[],
        "get_note" | "delete_note" | "get_session" | "delete_session" => &["id"],
        "get_backlinks" | "find_contradictions" => &["noteId"],
        "create_note" => &["note"],
        "update_note" | "update_session" => &["id", "update"],
        "update_settings" => &["update"],
        "search_notes" => &["query", "limit"],
        "list_link_suggestion_queue" | "get_vault_optimizer_inbox" => &["status", "limit"],
        "create_session" => &["session"],
        "send_prompt" => &["sessionId", "request"],
        "regenerate_response" => &["sessionId", "tileId", "modelId"],
        "record_canvas_feedback" => &["sessionId", "request"],
        "review_memory_digest_item" => &["id", "request"],
        "save_constitution_setup" => &["setup"],
        "list_twin_observations"
        | "list_twin_proposals"
        | "create_companion_capture"
        | "review_twin_proposal"
        | "get_twin_state_projection"
        | "rank_twin_attention"
        | "get_twin_event_timeline"
        | "discover_image_models"
        | "get_image_model_capability"
        | "generate_image"
        | "discard_generated_image_receipt"
        | "save_generated_image"
        | "load_generated_image"
        | "recall_relevant" => &["request"],
        "import_sync_envelopes" => &["bundle"],
        _ => &[],
    }
}

fn validate_invoke_arguments(command: &str, args: &Map<String, Value>) -> Result<(), String> {
    let expected = expected_argument_names(command);
    if args.keys().any(|name| !expected.contains(&name.as_str())) {
        return Err("invoke arguments contain an unknown field".to_string());
    }
    if command == "create_note" {
        let note = args
            .get("note")
            .and_then(Value::as_object)
            .ok_or_else(|| "create_note requires a note object".to_string())?;
        const NOTE_FIELDS: &[&str] = &[
            "title",
            "content",
            "relative_path",
            "aliases",
            "status",
            "tags",
            "schema_version",
            "migration_source",
            "optimizer_managed",
            "properties",
        ];
        if note
            .keys()
            .any(|name| !NOTE_FIELDS.contains(&name.as_str()))
        {
            return Err("create_note contains an unknown field".to_string());
        }
        if note
            .get("relative_path")
            .and_then(Value::as_str)
            .is_some_and(|path| std::path::Path::new(path).is_absolute())
        {
            return Err("create_note rejects absolute relative_path values".to_string());
        }
    }
    Ok(())
}

fn validate_harness_invoke_arguments(
    command: &str,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if command != "update_settings" {
        return Ok(());
    }
    let update = args
        .get("update")
        .and_then(Value::as_object)
        .ok_or_else(|| "E2E update_settings requires an update object".to_string())?;
    const COMPACT_SAFE_FIELDS: &[&str] = &["theme", "openrouter_api_key"];
    if update
        .keys()
        .any(|name| !COMPACT_SAFE_FIELDS.contains(&name.as_str()))
    {
        return Err(
            "E2E update_settings accepts only compact-safe theme and OpenRouter key fields"
                .to_string(),
        );
    }
    Ok(())
}

fn adapt_invoke_result(
    profile: TestProfile,
    command: &str,
    result: Value,
) -> Result<Value, String> {
    if profile != TestProfile::Android {
        return Ok(result);
    }
    match command {
        "get_settings" | "update_settings" => {
            let settings = serde_json::from_value(result)
                .map_err(|_| "E2E settings response is invalid".to_string())?;
            serde_json::to_value(crate::commands::settings::redact_settings_for_runtime(
                &settings,
                crate::models::runtime::RuntimeKind::Android,
            ))
            .map_err(|_| "E2E settings response serialization failed".to_string())
        }
        "get_settings_status" => {
            let status = serde_json::from_value(result)
                .map_err(|_| "E2E settings status response is invalid".to_string())?;
            serde_json::to_value(crate::commands::settings::redact_status_for_runtime(
                status,
                crate::models::runtime::RuntimeKind::Android,
            ))
            .map_err(|_| "E2E settings status response serialization failed".to_string())
        }
        _ => Ok(result),
    }
}

#[derive(Clone, Copy)]
struct InvokeHeaderValues<'a> {
    origin: &'a str,
    authorization: &'a str,
    profile: &'a str,
    device: &'a str,
    content_type: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InvokeHeaderError {
    Unauthorized,
    ForbiddenOrigin,
    UnsupportedMediaType,
    InvalidHarnessSelector,
}

fn validate_invoke_headers(
    config: &TestRuntimeConfig,
    headers: InvokeHeaderValues<'_>,
) -> Result<(TestProfile, TestDevice), InvokeHeaderError> {
    if headers.origin != config.origin {
        return Err(InvokeHeaderError::ForbiddenOrigin);
    }
    if headers.content_type != "application/json" {
        return Err(InvokeHeaderError::UnsupportedMediaType);
    }
    let supplied_token = headers
        .authorization
        .strip_prefix("Bearer ")
        .ok_or(InvokeHeaderError::Unauthorized)?;
    if !constant_time_equal(supplied_token.as_bytes(), config.bearer_token.as_bytes()) {
        return Err(InvokeHeaderError::Unauthorized);
    }
    let profile = TestProfile::parse(headers.profile)
        .map_err(|_| InvokeHeaderError::InvalidHarnessSelector)?;
    let device =
        TestDevice::parse(headers.device).map_err(|_| InvokeHeaderError::InvalidHarnessSelector)?;
    Ok((profile, device))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

const E2E_VAULT_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
const E2E_OPENROUTER_KEY_VERSION: &str = "123e4567-e89b-42d3-a456-4266141740f1";
const E2E_OPENROUTER_KEY: &str = "grafyn-e2e-key";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BufferedEvent {
    event: String,
    payload: Value,
}

struct MockCommandRuntime {
    _app: tauri::App<tauri::test::MockRuntime>,
    webview: tauri::WebviewWindow<tauri::test::MockRuntime>,
    state: crate::AppState,
    paths: crate::app_runtime::RuntimePaths,
    secret_store: Arc<crate::services::sync::secrets::MemorySecretStore>,
    events: Arc<Mutex<Vec<BufferedEvent>>>,
    canvas_events: crate::commands::canvas::BufferedCanvasEventSink,
}

pub struct TestRuntimeHarness {
    config: TestRuntimeConfig,
    secret_stores: BTreeMap<TestDevice, Arc<crate::services::sync::secrets::MemorySecretStore>>,
    devices: BTreeMap<TestDevice, MockCommandRuntime>,
    tainted_devices: BTreeSet<TestDevice>,
}

impl TestRuntimeHarness {
    pub async fn start(config: TestRuntimeConfig) -> Result<Self, String> {
        let secret_stores = [TestDevice::DeviceA, TestDevice::DeviceB]
            .into_iter()
            .map(|device| {
                (
                    device,
                    Arc::new(crate::services::sync::secrets::MemorySecretStore::default()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut runtime = Self {
            config,
            secret_stores,
            devices: BTreeMap::new(),
            tainted_devices: BTreeSet::new(),
        };
        for device in [TestDevice::DeviceA, TestDevice::DeviceB] {
            runtime.rebuild_device(device).await?;
        }
        runtime.pair_devices()?;
        Ok(runtime)
    }

    pub async fn restart(&mut self, device: TestDevice) -> Result<(), String> {
        self.ensure_device_is_not_tainted(device)?;
        if let Some(runtime) = self.devices.remove(&device) {
            runtime.shutdown()?;
        }
        self.rebuild_device(device).await
    }

    pub async fn invoke(&mut self, command: &str, args: Value) -> Result<Value, String> {
        self.invoke_selected(TestDevice::DeviceA, TestProfile::Desktop, command, args)
            .await
    }

    pub async fn invoke_selected(
        &mut self,
        device: TestDevice,
        profile: TestProfile,
        command: &str,
        args: Value,
    ) -> Result<Value, String> {
        if !is_command_allowed_for_profile(profile, command) {
            return Err("invoke command is unavailable".to_string());
        }
        let args = args
            .as_object()
            .cloned()
            .ok_or_else(|| "invoke args must be an object".to_string())?;
        validate_invoke_arguments(command, &args)?;
        validate_harness_invoke_arguments(command, &args)?;
        if command == "e2e_restart_runtime" {
            self.restart(device).await?;
            return Ok(Value::Bool(true));
        }
        let runtime = self
            .devices
            .get_mut(&device)
            .ok_or_else(|| "E2E device is unavailable".to_string())?;
        if command == "get_runtime_status" {
            return runtime.runtime_status(profile).await;
        }
        if command == "send_prompt" {
            let session_id = args
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "send_prompt requires sessionId".to_string())?
                .to_string();
            let request = serde_json::from_value(
                args.get("request")
                    .cloned()
                    .ok_or_else(|| "send_prompt requires request".to_string())?,
            )
            .map_err(|_| "send_prompt request is invalid".to_string())?;
            return crate::commands::canvas::send_prompt_with_sink(
                runtime.canvas_events.clone(),
                session_id,
                request,
                &runtime.state,
            )
            .await
            .map(Value::String)
            .map_err(|error| sanitize_command_error(&error));
        }
        if command == "regenerate_response" {
            let session_id = args
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "regenerate_response requires sessionId".to_string())?
                .to_string();
            let tile_id = args
                .get("tileId")
                .and_then(Value::as_str)
                .ok_or_else(|| "regenerate_response requires tileId".to_string())?
                .to_string();
            let model_id = args
                .get("modelId")
                .and_then(Value::as_str)
                .ok_or_else(|| "regenerate_response requires modelId".to_string())?
                .to_string();
            return crate::commands::canvas::regenerate_response_with_sink(
                runtime.canvas_events.clone(),
                session_id,
                tile_id,
                model_id,
                &runtime.state,
            )
            .await
            .map(|_| Value::Null)
            .map_err(|error| sanitize_command_error(&error));
        }
        let result = runtime.invoke(command, Value::Object(args))?;
        adapt_invoke_result(profile, command, result)
    }

    async fn invoke_for_response(
        &mut self,
        device: TestDevice,
        profile: TestProfile,
        command: &str,
        args: Value,
    ) -> Result<(Value, Vec<BufferedEvent>), String> {
        let result = self.invoke_selected(device, profile, command, args).await;
        let wait_for_canvas_terminal =
            matches!(command, "send_prompt" | "regenerate_response") && result.is_ok();
        let events = match self
            .take_response_events(device, wait_for_canvas_terminal)
            .await
        {
            Ok(events) => events,
            Err(error) => {
                if wait_for_canvas_terminal {
                    self.taint_device_after_canvas_terminal_failure(device)
                        .map_err(|quarantine_error| {
                            format!(
                                "{error}; E2E device quarantine after Canvas terminal failure failed: {quarantine_error}"
                            )
                        })?;
                }
                return Err(error);
            }
        };
        result.map(|value| (value, events))
    }

    fn take_events(&self, device: TestDevice) -> Result<Vec<BufferedEvent>, String> {
        self.devices
            .get(&device)
            .ok_or_else(|| "E2E device is unavailable".to_string())?
            .take_events()
    }

    async fn take_response_events(
        &self,
        device: TestDevice,
        wait_for_canvas_terminal: bool,
    ) -> Result<Vec<BufferedEvent>, String> {
        let deadline = tokio::time::Instant::now() + CANVAS_TERMINAL_TIMEOUT;
        let mut collected = Vec::new();
        loop {
            let batch = self.take_events(device)?;
            let terminal = batch.iter().any(is_canvas_terminal);
            if collected.len().saturating_add(batch.len()) > MAX_RESPONSE_EVENTS {
                return Err("E2E response event limit exceeded".to_string());
            }
            collected.extend(batch);
            if !wait_for_canvas_terminal || terminal {
                return Ok(collected);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(CANVAS_TERMINAL_TIMEOUT_ERROR.to_string());
            }
            let runtime = self
                .devices
                .get(&device)
                .ok_or_else(|| "E2E device is unavailable".to_string())?;
            tokio::time::timeout(remaining, runtime.canvas_events.wait_for_activity())
                .await
                .map_err(|_| CANVAS_TERMINAL_TIMEOUT_ERROR.to_string())?;
        }
    }

    async fn rebuild_device(&mut self, device: TestDevice) -> Result<(), String> {
        self.ensure_device_is_not_tainted(device)?;
        let secret_store = self
            .secret_stores
            .get(&device)
            .cloned()
            .ok_or_else(|| "E2E secret store is unavailable".to_string())?;
        let runtime = MockCommandRuntime::build(&self.config, device, secret_store).await?;
        self.devices.insert(device, runtime);
        Ok(())
    }

    fn ensure_device_is_not_tainted(&self, device: TestDevice) -> Result<(), String> {
        if self.tainted_devices.contains(&device) {
            return Err("E2E device is quarantined after a Canvas terminal failure".to_string());
        }
        Ok(())
    }

    fn taint_device_after_canvas_terminal_failure(
        &mut self,
        device: TestDevice,
    ) -> Result<(), String> {
        self.tainted_devices.insert(device);
        match self.devices.remove(&device) {
            Some(runtime) => runtime.shutdown(),
            None => Ok(()),
        }
    }

    fn pair_devices(&self) -> Result<(), String> {
        let first = self
            .devices
            .get(&TestDevice::DeviceA)
            .and_then(|runtime| runtime.state.sync_engine.clone())
            .ok_or_else(|| "E2E sync engine is unavailable".to_string())?;
        let second = self
            .devices
            .get(&TestDevice::DeviceB)
            .and_then(|runtime| runtime.state.sync_engine.clone())
            .ok_or_else(|| "E2E sync engine is unavailable".to_string())?;
        let (first_id, first_key) = first.local_device().map_err(|error| error.to_string())?;
        let (second_id, second_key) = second.local_device().map_err(|error| error.to_string())?;
        first
            .trust_device(second_id, second_key)
            .map_err(|error| error.to_string())?;
        second
            .trust_device(first_id, first_key)
            .map_err(|error| error.to_string())
    }
}

impl MockCommandRuntime {
    #[allow(deprecated)]
    fn shutdown(self) -> Result<(), String> {
        let Self {
            _app: app,
            webview,
            state,
            paths: _,
            secret_store: _,
            events: _,
            canvas_events: _,
        } = self;
        let managed = app
            .unmanage::<crate::AppState>()
            .ok_or_else(|| "E2E AppState was not managed".to_string())?;
        app.cleanup_before_exit();
        drop(webview);
        drop(state);
        drop(managed);
        drop(app);
        Ok(())
    }

    async fn build(
        config: &TestRuntimeConfig,
        device: TestDevice,
        secret_store: Arc<crate::services::sync::secrets::MemorySecretStore>,
    ) -> Result<Self, String> {
        let device_root = config.root.join(device.root_name());
        let paths = crate::app_runtime::RuntimePaths::desktop(
            device_root.join("config"),
            device_root.join("data"),
            device_root.join("vault"),
            device_root.join("cache"),
        );
        paths.prepare()?;
        install_fixed_identities(&paths, device)?;
        provision_e2e_secrets(&paths, device, secret_store.clone())?;
        let settings = crate::services::settings::SettingsService::load_for_e2e(
            paths.config_dir.join("settings.json"),
            paths.data_dir.clone(),
            paths.vault_dir.clone(),
            secret_store.clone(),
        )
        .map_err(|error| error.to_string())?;
        let state = crate::build_app_state(settings, None)?;
        *state.openrouter.write().await =
            crate::services::openrouter::OpenRouterService::new_for_e2e(
                E2E_OPENROUTER_KEY.to_string(),
                config.openrouter_url.clone(),
            )
            .map_err(|error| error.to_string())?;
        crate::warm_start_services_inner(None, &state, None).await?;

        let events = Arc::new(Mutex::new(Vec::new()));
        let canvas_events = crate::commands::canvas::BufferedCanvasEventSink::new();
        let app = tauri::test::mock_builder()
            .manage(state.clone())
            .invoke_handler(tauri::generate_handler![
                crate::commands::boot::get_boot_status,
                crate::commands::notes::list_notes,
                crate::commands::notes::get_note,
                crate::commands::notes::create_note,
                crate::commands::notes::update_note,
                crate::commands::notes::delete_note,
                crate::commands::search::search_notes,
                crate::commands::graph::get_full_graph,
                crate::commands::graph::get_backlinks,
                crate::commands::canvas::list_sessions,
                crate::commands::canvas::get_session,
                crate::commands::canvas::create_session,
                crate::commands::canvas::update_session,
                crate::commands::canvas::delete_session,
                crate::commands::canvas::get_available_models,
                crate::commands::twin::get_twin_review,
                crate::commands::twin::run_twin_inference,
                crate::commands::twin::record_canvas_feedback,
                crate::commands::twin::list_decision_episodes,
                crate::commands::twin::get_decision_mirror_config,
                crate::commands::twin::list_memory_digest,
                crate::commands::twin::review_memory_digest_item,
                crate::commands::twin::list_constitution_items,
                crate::commands::twin::list_action_gaps,
                crate::commands::twin::get_constitution_setup,
                crate::commands::twin::save_constitution_setup,
                crate::commands::twin_state::list_twin_observations,
                crate::commands::twin_state::list_twin_proposals,
                crate::commands::twin_state::create_companion_capture,
                crate::commands::twin_state::review_twin_proposal,
                crate::commands::twin_state::get_twin_state_projection,
                crate::commands::twin_state::rank_twin_attention,
                crate::commands::twin_state::get_twin_event_timeline,
                crate::commands::image_generation::discover_image_models,
                crate::commands::image_generation::get_image_model_capability,
                crate::commands::image_generation::generate_image,
                crate::commands::image_generation::discard_generated_image_receipt,
                crate::commands::image_generation::save_generated_image,
                crate::commands::image_generation::load_generated_image,
                crate::commands::memory::recall_relevant,
                crate::commands::memory::find_contradictions,
                crate::commands::settings::get_settings,
                crate::commands::settings::get_settings_status,
                crate::commands::settings::update_settings,
                crate::commands::settings::get_openrouter_status,
                crate::commands::migration::get_vault_optimizer_status,
                crate::commands::migration::get_vault_optimizer_inbox,
                crate::commands::zettelkasten::list_link_suggestion_queue,
                crate::commands::zettelkasten::get_link_discovery_status,
                crate::commands::mcp::get_mcp_status,
                crate::commands::sync::get_sync_status,
                crate::commands::sync::list_sync_conflicts,
                crate::commands::sync::export_sync_outbox,
                crate::commands::sync::import_sync_envelopes,
                crate::commands::sync::rebuild_sync_state,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| format!("failed to build E2E IPC runtime: {error}"))?;
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .map_err(|error| format!("failed to build E2E IPC webview: {error}"))?;
        for event_name in ["canvas-stream", "boot-status"] {
            let pending = events.clone();
            app.listen(event_name, move |event| {
                let payload = serde_json::from_str(event.payload())
                    .unwrap_or_else(|_| Value::String(event.payload().to_string()));
                if let Ok(mut pending) = pending.lock() {
                    if pending.len() < 1024 {
                        pending.push(BufferedEvent {
                            event: event_name.to_string(),
                            payload,
                        });
                    }
                }
            });
        }
        Ok(Self {
            _app: app,
            webview,
            state,
            paths,
            secret_store,
            events,
            canvas_events,
        })
    }

    fn invoke(&mut self, command: &str, args: Value) -> Result<Value, String> {
        tauri::test::get_ipc_response(
            &self.webview,
            tauri::webview::InvokeRequest {
                cmd: command.to_string(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost"
                    .parse()
                    .expect("fixed Tauri test URL"),
                body: tauri::ipc::InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
        .and_then(|body| {
            body.deserialize::<Value>()
                .map_err(|error| Value::String(error.to_string()))
        })
        .map_err(|error| sanitize_command_error(&error.to_string()))
    }

    fn take_events(&self) -> Result<Vec<BufferedEvent>, String> {
        let mut events = {
            let mut buffered = self
                .events
                .lock()
                .map_err(|_| "E2E event buffer is unavailable".to_string())?;
            std::mem::take(&mut *buffered)
        };
        let canvas = self.canvas_events.drain();
        if canvas.dropped_events != 0 {
            return Err("E2E Canvas event buffer exceeded its fixed limit".to_string());
        }
        if canvas.total_bytes > 1024 * 1024 {
            return Err("E2E Canvas event buffer exceeded its byte limit".to_string());
        }
        for payload in canvas.events {
            events.push(BufferedEvent {
                event: "canvas-stream".to_string(),
                payload: serde_json::to_value(payload)
                    .map_err(|_| "E2E Canvas event serialization failed".to_string())?,
            });
        }
        Ok(events)
    }

    async fn runtime_status(&self, profile: TestProfile) -> Result<Value, String> {
        let kind = match profile {
            TestProfile::Android => crate::models::runtime::RuntimeKind::Android,
            TestProfile::Desktop => crate::models::runtime::RuntimeKind::Desktop,
        };
        let bootstrap = crate::app_runtime::RuntimeBootstrap::new(
            kind,
            self.paths.clone(),
            self.secret_store.clone(),
            crate::models::runtime::RuntimeFeatureStatusV1::ready(),
            crate::models::runtime::RuntimeFeatureStatusV1::unavailable(
                "e2e_native_share_unavailable",
                "Native image sharing is unavailable in the E2E runtime.",
            ),
        );
        let canonical = self.state.mutation_coordinator.is_some()
            && self.state.mutation_startup_error.read().await.is_none();
        let status = bootstrap.status_for_state(canonical, self.state.sync_engine.is_some());
        let mut status = crate::commands::runtime::runtime_status_inner(&status, &self.state).await;
        if profile == TestProfile::Android && canonical && status.secure_secrets.is_ready() {
            status.capabilities.image_generation = true;
            status.capabilities.native_image_share = false;
        }
        serde_json::to_value(status).map_err(|_| "runtime status serialization failed".to_string())
    }
}

fn install_fixed_identities(
    paths: &crate::app_runtime::RuntimePaths,
    device: TestDevice,
) -> Result<(), String> {
    let vault_root = crate::services::twin_events::AnchoredRoot::open(&paths.vault_dir)
        .map_err(|error| error.to_string())?;
    install_exact_json(
        &vault_root,
        "_grafyn/vault.json",
        serde_json::json!({ "schema_version": 1, "vault_id": E2E_VAULT_ID }),
    )?;
    let data_root = crate::services::twin_events::AnchoredRoot::open(&paths.data_dir)
        .map_err(|error| error.to_string())?;
    install_exact_json(
        &data_root,
        "twin/events/writer-v1.json",
        serde_json::json!({
            "schema_version": 1,
            "device_id": device.writer_device_id(),
            "actor_id": "owner"
        }),
    )
}

fn install_exact_json(
    root: &crate::services::twin_events::AnchoredRoot,
    key: &str,
    value: Value,
) -> Result<(), String> {
    let mut expected = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
    expected.push(b'\n');
    match root
        .read_bounded(key, 4096)
        .map_err(|error| error.to_string())?
    {
        Some(bytes) if bytes == expected => Ok(()),
        Some(_) => Err("E2E identity does not match its fixed authority".to_string()),
        None => {
            root.put_atomic(key, &expected)
                .map_err(|error| error.to_string())?;
            match root
                .read_bounded(key, 4096)
                .map_err(|error| error.to_string())?
            {
                Some(bytes) if bytes == expected => Ok(()),
                _ => Err("E2E identity failed durable readback".to_string()),
            }
        }
    }
}

fn provision_e2e_secrets(
    paths: &crate::app_runtime::RuntimePaths,
    device: TestDevice,
    store: Arc<crate::services::sync::secrets::MemorySecretStore>,
) -> Result<(), String> {
    use crate::services::sync::secrets::{SecretAccount, SecretStoreError};
    let root_account =
        SecretAccount::sync_vault_root(E2E_VAULT_ID).map_err(|error| error.to_string())?;
    put_or_verify_secret(store.as_ref(), &root_account, &[0x47; 32])?;
    put_or_verify_secret(
        store.as_ref(),
        &SecretAccount::sync_device_ed25519(),
        &device.signing_seed(),
    )?;
    let openrouter_account = SecretAccount::openrouter_key(E2E_OPENROUTER_KEY_VERSION)
        .map_err(|error| error.to_string())?;
    put_or_verify_secret(
        store.as_ref(),
        &openrouter_account,
        E2E_OPENROUTER_KEY.as_bytes(),
    )?;
    let transition = crate::services::root_transition::RootTransitionStore::new(
        &paths.data_dir,
        paths.config_dir.join("settings.json"),
        store,
    )
    .map_err(|error| error.to_string())?;
    transition
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Versioned,
            Some(E2E_OPENROUTER_KEY_VERSION),
        )
        .map_err(|error| error.to_string())?;
    let _ = SecretStoreError::AlreadyExists;
    Ok(())
}

fn put_or_verify_secret(
    store: &crate::services::sync::secrets::MemorySecretStore,
    account: &crate::services::sync::secrets::SecretAccount,
    expected: &[u8],
) -> Result<(), String> {
    use crate::services::sync::secrets::{SecretBytes, SecretStore, SecretStoreError};
    let secret = SecretBytes::from_slice(expected).map_err(|error| error.to_string())?;
    match store.put(account, &secret) {
        Ok(()) => Ok(()),
        Err(SecretStoreError::AlreadyExists) => match store.get(account) {
            Ok(Some(actual)) if actual.expose() == expected => Ok(()),
            _ => Err("E2E secret authority mismatch".to_string()),
        },
        Err(error) => Err(error.to_string()),
    }
}

fn sanitize_command_error(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len().min(512));
    let mut pending_space = false;
    for character in raw.chars().take(512) {
        if character.is_control() || character.is_whitespace() {
            pending_space = !sanitized.is_empty();
        } else {
            if pending_space {
                sanitized.push(' ');
                pending_space = false;
            }
            sanitized.push(character);
        }
    }
    if sanitized.is_empty() {
        "Command failed".to_string()
    } else {
        sanitized
    }
}

fn is_canvas_terminal(event: &BufferedEvent) -> bool {
    event.event == "canvas-stream"
        && event.payload.get("type").and_then(Value::as_str) == Some("session_saved")
}

fn encode_success_body(result: Value, events: Vec<BufferedEvent>) -> Result<Vec<u8>, String> {
    if events.len() > MAX_RESPONSE_EVENTS {
        return Err("E2E response event limit exceeded".to_string());
    }
    let bytes = serde_json::to_vec(&serde_json::json!({
        "result": result,
        "events": events,
    }))
    .map_err(|_| "E2E response serialization failed".to_string())?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("E2E response exceeds its fixed byte limit".to_string());
    }
    Ok(bytes)
}

fn encode_error_body(code: &str, message: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "error": {
            "code": code,
            "message": message,
        }
    }))
    .unwrap_or_else(|_| {
        br#"{"error":{"code":"runtime_failed","message":"Grafyn E2E runtime failed."}}"#.to_vec()
    })
}

type TestHttpBody = Full<Bytes>;

pub async fn run_from_env() -> Result<(), String> {
    let config = TestRuntimeConfig::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr())
        .await
        .map_err(|_| "Grafyn E2E runtime could not bind its loopback port".to_string())?;
    let harness = Rc::new(tokio::sync::Mutex::new(
        TestRuntimeHarness::start(config.clone()).await?,
    ));
    loop {
        let (stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "Grafyn E2E runtime could not accept a loopback request".to_string())?;
        if peer.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) {
            continue;
        }
        let harness = Rc::clone(&harness);
        let request_config = config.clone();
        let service = service_fn(move |request| {
            let harness = Rc::clone(&harness);
            let config = request_config.clone();
            async move { Ok::<_, Infallible>(handle_http_request(request, harness, config).await) }
        });
        let _ = http1::Builder::new()
            .keep_alive(false)
            .serve_connection(TokioIo::new(stream), service)
            .await;
    }
}

async fn handle_http_request(
    request: Request<Incoming>,
    harness: Rc<tokio::sync::Mutex<TestRuntimeHarness>>,
    config: TestRuntimeConfig,
) -> Response<TestHttpBody> {
    if request.uri().query().is_some() {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Grafyn E2E route was not found.",
            None,
        );
    }
    if request.method() == Method::GET && request.uri().path() == "/health" {
        return json_response(StatusCode::OK, br#"{"status":"ready"}"#.to_vec(), None);
    }
    if request.method() == Method::OPTIONS && request.uri().path() == "/invoke" {
        return preflight_response(&request, &config);
    }
    if request.uri().path() != "/invoke" {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Grafyn E2E route was not found.",
            None,
        );
    }
    if request.method() != Method::POST {
        let mut response = error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Grafyn E2E route requires POST.",
            None,
        );
        response.headers_mut().insert(
            header::ALLOW,
            hyper::header::HeaderValue::from_static("POST, OPTIONS"),
        );
        return response;
    }

    let header_values = match invoke_header_values(request.headers()) {
        Ok(values) => values,
        Err(error) => return header_error_response(error, None),
    };
    let (profile, device) = match validate_invoke_headers(&config, header_values) {
        Ok(selection) => selection,
        Err(error) => {
            let origin =
                (error != InvokeHeaderError::ForbiddenOrigin).then_some(config.origin.as_str());
            return header_error_response(error, origin);
        }
    };
    let origin = config.origin.clone();
    if declared_body_too_large(request.headers()) {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "Grafyn E2E invoke request exceeds 1 MiB.",
            Some(&origin),
        );
    }
    let body = match read_bounded_body(request.into_body()).await {
        Ok(body) => body,
        Err(BodyReadError::TooLarge) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                "Grafyn E2E invoke request exceeds 1 MiB.",
                Some(&origin),
            )
        }
        Err(BodyReadError::Invalid) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invoke_failed",
                "Grafyn E2E invoke request is invalid.",
                Some(&origin),
            )
        }
    };
    let envelope = match parse_invoke_envelope(&body) {
        Ok(envelope) => envelope,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invoke_failed",
                "Grafyn E2E invoke request is invalid.",
                Some(&origin),
            )
        }
    };
    let mut harness = harness.lock().await;
    let (result, events) = match harness
        .invoke_for_response(
            device,
            profile,
            &envelope.command,
            Value::Object(envelope.args),
        )
        .await
    {
        Ok(result) => result,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invoke_failed",
                "Grafyn command failed.",
                Some(&origin),
            )
        }
    };
    match encode_success_body(result, events) {
        Ok(body) => json_response(StatusCode::OK, body, Some(&origin)),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "response_too_large",
            "Grafyn E2E response exceeded its fixed limit.",
            Some(&origin),
        ),
    }
}

fn invoke_header_values(headers: &HeaderMap) -> Result<InvokeHeaderValues<'_>, InvokeHeaderError> {
    Ok(InvokeHeaderValues {
        origin: single_header(headers, "origin").ok_or(InvokeHeaderError::ForbiddenOrigin)?,
        authorization: single_header(headers, "authorization")
            .ok_or(InvokeHeaderError::Unauthorized)?,
        profile: single_header(headers, "x-grafyn-e2e-profile")
            .ok_or(InvokeHeaderError::InvalidHarnessSelector)?,
        device: single_header(headers, "x-grafyn-e2e-device")
            .ok_or(InvokeHeaderError::InvalidHarnessSelector)?,
        content_type: single_header(headers, "content-type")
            .ok_or(InvokeHeaderError::UnsupportedMediaType)?,
    })
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

fn declared_body_too_large(headers: &HeaderMap) -> bool {
    single_header(headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_REQUEST_BYTES)
}

enum BodyReadError {
    TooLarge,
    Invalid,
}

async fn read_bounded_body(mut body: Incoming) -> Result<Vec<u8>, BodyReadError> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| BodyReadError::Invalid)?;
        let data = frame.into_data().map_err(|_| BodyReadError::Invalid)?;
        let next_len = bytes
            .len()
            .checked_add(data.len())
            .ok_or(BodyReadError::TooLarge)?;
        if next_len > MAX_REQUEST_BYTES {
            return Err(BodyReadError::TooLarge);
        }
        bytes.extend_from_slice(&data);
    }
    Ok(bytes)
}

fn preflight_response(
    request: &Request<Incoming>,
    config: &TestRuntimeConfig,
) -> Response<TestHttpBody> {
    let origin = match single_header(request.headers(), "origin") {
        Some(origin) if origin == config.origin => origin,
        _ => {
            return error_response(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                "Grafyn E2E origin was rejected.",
                None,
            )
        }
    };
    let method_ok =
        single_header(request.headers(), "access-control-request-method") == Some("POST");
    let headers_ok = single_header(request.headers(), "access-control-request-headers")
        .is_some_and(exact_cors_request_headers);
    if !method_ok || !headers_ok {
        return error_response(
            StatusCode::FORBIDDEN,
            "forbidden_preflight",
            "Grafyn E2E preflight was rejected.",
            Some(origin),
        );
    }
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, CORS_REQUEST_HEADERS)
        .header(header::VARY, "Origin")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONNECTION, "close")
        .body(Full::new(Bytes::new()))
        .expect("fixed preflight response headers are valid")
}

fn exact_cors_request_headers(value: &str) -> bool {
    let mut supplied = value.split(',').map(str::trim).collect::<Vec<_>>();
    if supplied.len() != 4
        || supplied
            .iter()
            .any(|value| value.to_ascii_lowercase() != *value)
    {
        return false;
    }
    supplied.sort_unstable();
    supplied
        == [
            "authorization",
            "content-type",
            "x-grafyn-e2e-device",
            "x-grafyn-e2e-profile",
        ]
}

fn header_error_response(error: InvokeHeaderError, origin: Option<&str>) -> Response<TestHttpBody> {
    match error {
        InvokeHeaderError::Unauthorized => error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Grafyn E2E authorization was rejected.",
            origin,
        ),
        InvokeHeaderError::ForbiddenOrigin => error_response(
            StatusCode::FORBIDDEN,
            "forbidden_origin",
            "Grafyn E2E origin was rejected.",
            None,
        ),
        InvokeHeaderError::UnsupportedMediaType => error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Grafyn E2E invoke requires application/json.",
            origin,
        ),
        InvokeHeaderError::InvalidHarnessSelector => error_response(
            StatusCode::BAD_REQUEST,
            "invoke_failed",
            "Grafyn E2E harness selector was rejected.",
            origin,
        ),
    }
}

fn error_response(
    status: StatusCode,
    code: &str,
    message: &str,
    origin: Option<&str>,
) -> Response<TestHttpBody> {
    json_response(status, encode_error_body(code, message), origin)
}

fn json_response(
    status: StatusCode,
    bytes: Vec<u8>,
    origin: Option<&str>,
) -> Response<TestHttpBody> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .header(header::CONNECTION, "close");
    if let Some(origin) = origin {
        builder = builder
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
            .header(header::VARY, "Origin");
    }
    builder
        .body(Full::new(Bytes::from(bytes)))
        .expect("fixed JSON response headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(root: &std::path::Path) -> TestRuntimeConfig {
        config_with_openrouter(root, "http://127.0.0.1:18891/api/v1")
    }

    fn config_with_openrouter(root: &std::path::Path, openrouter_url: &str) -> TestRuntimeConfig {
        TestRuntimeConfig::from_values(TestRuntimeConfigInput {
            root: root.to_path_buf(),
            port: 18890,
            origin: "http://127.0.0.1:5173".to_string(),
            bearer_token: "a".repeat(64),
            openrouter_url: openrouter_url.to_string(),
        })
        .unwrap()
    }

    async fn spawn_openrouter_text_stub(
        expected_requests: usize,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..expected_requests {
                let (stream, _) = listener.accept().await.unwrap();
                let service = service_fn(|_request: Request<Incoming>| async move {
                    let body = concat!(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"E2E answer\"}}]}\n\n",
                        "data: [DONE]\n\n"
                    );
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, "text/event-stream")
                            .header(header::CONNECTION, "close")
                            .body(Full::new(Bytes::from_static(body.as_bytes())))
                            .unwrap(),
                    )
                });
                http1::Builder::new()
                    .keep_alive(false)
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                    .unwrap();
            }
        });
        (format!("http://{address}/api/v1"), server)
    }

    async fn spawn_hanging_openrouter_stub() -> (
        String,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (release, released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = released.await;
        });
        (format!("http://{address}/api/v1"), release, server)
    }

    #[test]
    fn configuration_accepts_only_fixed_loopback_authorities_and_strong_ephemeral_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let valid = config(temp.path());
        assert_eq!(valid.bind_addr().to_string(), "127.0.0.1:18890");

        for (origin, token, upstream) in [
            (
                "http://localhost:5173",
                "a".repeat(64),
                "http://127.0.0.1:18891/api/v1",
            ),
            (
                "http://127.0.0.1:5173/path",
                "a".repeat(64),
                "http://127.0.0.1:18891/api/v1",
            ),
            (
                "http://127.0.0.1:5173",
                "A".repeat(64),
                "http://127.0.0.1:18891/api/v1",
            ),
            (
                "http://127.0.0.1:5173",
                "a".repeat(63),
                "http://127.0.0.1:18891/api/v1",
            ),
            (
                "http://127.0.0.1:5173",
                "a".repeat(64),
                "https://openrouter.ai/api/v1",
            ),
            (
                "http://127.0.0.1:5173",
                "a".repeat(64),
                "http://user@127.0.0.1:18891/api/v1",
            ),
        ] {
            let result = TestRuntimeConfig::from_values(TestRuntimeConfigInput {
                root: temp.path().to_path_buf(),
                port: 18890,
                origin: origin.to_string(),
                bearer_token: token,
                openrouter_url: upstream.to_string(),
            });
            assert!(
                result.is_err(),
                "accepted unsafe config: {origin} {upstream}"
            );
        }
    }

    #[test]
    fn configuration_requires_an_empty_or_previously_marked_runtime_root() {
        let occupied = tempfile::tempdir().unwrap();
        let witness = occupied.path().join("owner-data.txt");
        std::fs::write(&witness, b"must remain untouched").unwrap();
        let rejected = TestRuntimeConfig::from_values(TestRuntimeConfigInput {
            root: occupied.path().to_path_buf(),
            port: 18890,
            origin: "http://127.0.0.1:5173".to_string(),
            bearer_token: "a".repeat(64),
            openrouter_url: "http://127.0.0.1:18891/api/v1".to_string(),
        });
        assert!(rejected.is_err());
        assert_eq!(std::fs::read(&witness).unwrap(), b"must remain untouched");
        assert!(!occupied.path().join(".grafyn-e2e-owned-v1").exists());

        let owned = tempfile::tempdir().unwrap();
        config(owned.path());
        let marker = owned.path().join(".grafyn-e2e-owned-v1");
        assert!(marker.is_file());
        std::fs::write(owned.path().join("later-test-data.txt"), b"owned").unwrap();
        config(owned.path());
    }

    #[test]
    fn invoke_envelope_is_strict_bounded_and_allowlisted() {
        assert!(matches!(
            parse_invoke_envelope(br#"{"command":"list_notes","args":{}}"#).unwrap(),
            InvokeEnvelope { command, .. } if command == "list_notes"
        ));
        assert!(parse_invoke_envelope(
            br#"{"command":"list_notes","args":{},"path":"C:/outside"}"#
        )
        .is_err());
        assert!(parse_invoke_envelope(br#"{"command":"list_notes"}"#).is_err());
        assert!(parse_invoke_envelope(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
        assert!(is_allowlisted_command("list_notes"));
        for denied in [
            "pick_vault_folder",
            "preview_import",
            "apply_markdown_migration",
            "get_mcp_config_snippet",
            "restart_app",
            "e2e_read_file",
        ] {
            assert!(
                !is_allowlisted_command(denied),
                "unsafe command was allowlisted: {denied}"
            );
        }
    }

    #[test]
    fn contradiction_lookup_accepts_only_the_production_tauri_argument() {
        assert!(is_allowlisted_command("find_contradictions"));
        let accepted = json!({ "noteId": "note-1" }).as_object().unwrap().clone();
        assert!(validate_invoke_arguments("find_contradictions", &accepted).is_ok());

        let rejected = json!({ "note_id": "note-1" }).as_object().unwrap().clone();
        assert!(validate_invoke_arguments("find_contradictions", &rejected).is_err());
    }

    #[tokio::test]
    async fn real_ipc_state_is_root_isolated_and_restarts_over_durable_data() {
        let owned = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_witness = outside.path().join("untouched.txt");
        std::fs::write(&outside_witness, b"owner bytes").unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();

        let created = runtime
            .invoke(
                "create_note",
                json!({
                    "note": {
                        "title": "Durable companion note",
                        "content": "Captured before restart",
                        "tags": ["e2e"]
                    }
                }),
            )
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap().to_string();

        assert!(runtime
            .invoke(
                "create_note",
                json!({
                    "note": {
                        "title": "Caller path rejected",
                        "content": "No escape",
                        "relative_path": outside.path().join("escape.md")
                    }
                }),
            )
            .await
            .is_err());
        runtime.restart(TestDevice::DeviceA).await.unwrap();

        let reopened = runtime
            .invoke("get_note", json!({ "id": id }))
            .await
            .unwrap();
        assert_eq!(reopened["content"], "Captured before restart");
        assert_eq!(std::fs::read(&outside_witness).unwrap(), b"owner bytes");
        assert!(!outside.path().join("escape.md").exists());
        assert!(owned.path().join("device-a/vault").is_dir());
        assert!(owned.path().join("device-a/data").is_dir());
    }

    #[tokio::test]
    async fn android_profile_advertises_harness_image_generation_without_native_share() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let status = runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Android,
                "get_runtime_status",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(status["runtime"], "android");
        assert_eq!(status["vault"]["kind"], "app_private");
        assert_eq!(status["capabilities"]["imageGeneration"], true);
        assert_eq!(status["capabilities"]["nativeImageShare"], false);
    }

    #[tokio::test]
    async fn android_profile_rejects_desktop_commands_but_keeps_explicit_harness_controls() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();

        assert!(runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Android,
                "list_notes",
                json!({}),
            )
            .await
            .is_ok());
        for desktop_only in [
            "search_notes",
            "get_full_graph",
            "find_contradictions",
            "run_twin_inference",
            "get_mcp_status",
            "get_vault_optimizer_status",
        ] {
            assert!(is_command_allowed_for_profile(
                TestProfile::Desktop,
                desktop_only
            ));
            assert!(!is_command_allowed_for_profile(
                TestProfile::Android,
                desktop_only
            ));
            assert!(
                runtime
                    .invoke_selected(
                        TestDevice::DeviceA,
                        TestProfile::Android,
                        desktop_only,
                        json!({}),
                    )
                    .await
                    .is_err(),
                "Android unexpectedly invoked {desktop_only}"
            );
        }

        assert!(runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Android,
                "export_sync_outbox",
                json!({}),
            )
            .await
            .is_ok());
        assert!(runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Android,
                "e2e_restart_runtime",
                json!({}),
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn desktop_profile_exposes_read_surfaces_with_strict_arguments() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let note = runtime
            .invoke(
                "create_note",
                json!({
                    "note": {
                        "title": "Contradiction lookup fixture",
                        "content": "The desktop harness must route memory lookups."
                    }
                }),
            )
            .await
            .unwrap();
        let note_id = note["id"].as_str().unwrap().to_string();

        for (command, args) in [
            (
                "list_link_suggestion_queue",
                json!({ "status": "pending", "limit": 25 }),
            ),
            ("get_link_discovery_status", json!({})),
            (
                "get_vault_optimizer_inbox",
                json!({ "status": null, "limit": 20 }),
            ),
            ("get_backlinks", json!({ "noteId": "missing-note" })),
            ("find_contradictions", json!({ "noteId": note_id.clone() })),
        ] {
            let response = runtime
                .invoke_selected(
                    TestDevice::DeviceA,
                    TestProfile::Desktop,
                    command,
                    args.clone(),
                )
                .await;
            assert!(
                response.is_ok(),
                "Desktop failed to invoke {command}: {response:?}"
            );

            let mut unknown_args = args.as_object().unwrap().clone();
            unknown_args.insert("unexpected".to_string(), Value::Bool(true));
            let error = runtime
                .invoke_selected(
                    TestDevice::DeviceA,
                    TestProfile::Desktop,
                    command,
                    Value::Object(unknown_args),
                )
                .await
                .unwrap_err();
            assert!(error.contains("unknown field"), "{command}: {error}");

            assert!(
                runtime
                    .invoke_selected(TestDevice::DeviceA, TestProfile::Android, command, args,)
                    .await
                    .is_err(),
                "Android unexpectedly invoked {command}"
            );
        }
    }

    #[tokio::test]
    async fn android_profile_exposes_only_compact_safe_settings_updates() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();

        assert!(is_command_allowed_for_profile(
            TestProfile::Android,
            "update_settings"
        ));
        let (android_before, _) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "get_settings",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(android_before["vault_path"], Value::Null);
        assert!(android_before.get("openrouter_api_key").is_none());
        let (android_status, _) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "get_settings_status",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(android_status["vault_path"], Value::Null);

        let (desktop_before, _) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "get_settings",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::canonicalize(desktop_before["vault_path"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(owned.path().join("device-a/vault")).unwrap()
        );
        assert!(desktop_before.get("openrouter_api_key").is_none());

        let outside = tempfile::tempdir().unwrap();
        let outside_witness = outside.path().join("owner-data.txt");
        std::fs::write(&outside_witness, b"must remain untouched").unwrap();
        let desktop_error = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "update_settings",
                json!({
                    "update": { "vault_path": outside.path().to_string_lossy() }
                }),
            )
            .await
            .unwrap_err();
        assert!(desktop_error.contains("compact-safe"));
        assert_eq!(
            std::fs::read(&outside_witness).unwrap(),
            b"must remain untouched"
        );
        let (desktop_after, _) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "get_settings",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::canonicalize(desktop_after["vault_path"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(owned.path().join("device-a/vault")).unwrap()
        );

        let (updated, events) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "update_settings",
                json!({
                    "update": {
                        "theme": "dark",
                        "openrouter_api_key": "mobile-e2e-key"
                    }
                }),
            )
            .await
            .unwrap();
        assert_eq!(updated["theme"], "dark");
        assert_eq!(updated["vault_path"], Value::Null);
        assert!(updated.get("openrouter_api_key").is_none());
        assert!(events.is_empty());

        for forbidden in [
            "vault_path",
            "setup_completed",
            "mcp_enabled",
            "llm_model",
            "twin_llm_provider",
            "ollama_base_url",
            "ollama_model",
            "smart_web_search",
            "background_link_discovery_enabled",
            "background_link_discovery_llm_enabled",
            "background_vault_optimizer_enabled",
            "background_vault_optimizer_llm_enabled",
            "background_vault_optimizer_budget_monthly",
            "background_vault_optimizer_max_daily_writes",
            "background_vault_optimizer_edit_mode",
            "background_vault_optimizer_program_enabled",
            "vault_optimizer_program_path",
            "canvas_model_presets",
            "unexpected",
        ] {
            let mut update = Map::new();
            update.insert(forbidden.to_string(), Value::Null);
            let error = runtime
                .invoke_for_response(
                    TestDevice::DeviceA,
                    TestProfile::Android,
                    "update_settings",
                    json!({ "update": Value::Object(update) }),
                )
                .await
                .unwrap_err();
            assert!(error.contains("compact-safe"), "{forbidden}: {error}");
        }
        assert!(runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "update_settings",
                json!({ "update": { "theme": "light" }, "unexpected": true }),
            )
            .await
            .unwrap_err()
            .contains("unknown field"));
    }

    #[tokio::test]
    async fn android_regeneration_waits_for_its_persisted_session_boundary() {
        let owned = tempfile::tempdir().unwrap();
        let (openrouter_url, openrouter_stub) = spawn_openrouter_text_stub(2).await;
        let mut runtime =
            TestRuntimeHarness::start(config_with_openrouter(owned.path(), &openrouter_url))
                .await
                .unwrap();
        assert!(is_command_allowed_for_profile(
            TestProfile::Android,
            "regenerate_response"
        ));

        let (session, _) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "create_session",
                json!({ "session": { "title": "Regeneration boundary" } }),
            )
            .await
            .unwrap();
        let session_id = session["id"].as_str().unwrap();
        let model_id = "openai/e2e-missing";
        let (tile_id, initial_events) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "send_prompt",
                json!({
                    "sessionId": session_id,
                    "request": { "prompt": "Generate once", "models": [model_id] }
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            initial_events.last().unwrap().payload["type"],
            "session_saved"
        );

        let (_, regeneration_events) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "regenerate_response",
                json!({
                    "sessionId": session_id,
                    "tileId": tile_id,
                    "modelId": model_id
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            regeneration_events.last().unwrap().payload["type"],
            "session_saved"
        );
        assert!(runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "regenerate_response",
                json!({
                    "sessionId": session_id,
                    "tileId": tile_id,
                    "modelId": model_id,
                    "unexpected": true
                }),
            )
            .await
            .unwrap_err()
            .contains("unknown field"));
        let (_, next_events) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Android,
                "list_notes",
                json!({}),
            )
            .await
            .unwrap();
        assert!(next_events.is_empty());
        openrouter_stub.await.unwrap();
    }

    #[tokio::test]
    async fn real_ipc_saves_constitution_setup() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let saved = runtime
            .invoke(
                "save_constitution_setup",
                json!({
                    "setup": {
                        "twin_name": "Grafyn E2E Twin",
                        "twin_role": "A reviewed memory companion",
                        "source_boundaries": [],
                        "values": [],
                        "tastes": [],
                        "constraints": [],
                        "somatic_cues": [],
                        "action_tendencies": [],
                        "updated_at": null
                    }
                }),
            )
            .await
            .unwrap();

        assert_eq!(saved["twin_name"], "Grafyn E2E Twin");
        assert_eq!(saved["twin_role"], "A reviewed memory companion");
    }

    #[tokio::test]
    async fn desktop_read_only_status_commands_use_real_ipc_and_reject_arguments() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();

        for command in ["get_vault_optimizer_status", "get_mcp_status"] {
            let status = runtime.invoke(command, json!({})).await.unwrap();
            assert!(status.is_object(), "{command} returned a non-object status");
            assert!(runtime
                .invoke(command, json!({ "path": "C:/caller-controlled" }))
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn real_warm_start_publishes_ready_boot_status_without_an_event_window() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let status = runtime.invoke("get_boot_status", json!({})).await.unwrap();

        assert_eq!(status["phase"], "ready");
        assert_eq!(status["ready"], true);
        assert_eq!(status["error"], Value::Null);
    }

    #[test]
    fn invoke_headers_require_exact_origin_bearer_profile_and_device() {
        let temp = tempfile::tempdir().unwrap();
        let config = config(temp.path());
        let authorization = format!("Bearer {}", "a".repeat(64));
        let valid = InvokeHeaderValues {
            origin: "http://127.0.0.1:5173",
            authorization: &authorization,
            profile: "android",
            device: "device-a",
            content_type: "application/json",
        };
        assert_eq!(
            validate_invoke_headers(&config, valid).unwrap(),
            (TestProfile::Android, TestDevice::DeviceA)
        );

        for invalid in [
            InvokeHeaderValues {
                origin: "http://127.0.0.1:5174",
                ..valid
            },
            InvokeHeaderValues {
                authorization: "Bearer wrong",
                ..valid
            },
            InvokeHeaderValues {
                profile: "ios",
                ..valid
            },
            InvokeHeaderValues {
                device: "device-c",
                ..valid
            },
            InvokeHeaderValues {
                content_type: "text/plain",
                ..valid
            },
        ] {
            assert!(validate_invoke_headers(&config, invalid).is_err());
        }
    }

    #[test]
    fn http_envelopes_are_exact_bounded_and_canvas_terminal_is_explicit() {
        let event = BufferedEvent {
            event: "canvas-stream".to_string(),
            payload: json!({ "type": "session_saved", "session_id": "session-1" }),
        };
        let encoded = encode_success_body(json!({ "ok": true }), vec![event.clone()]).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encoded).unwrap(),
            json!({
                "result": { "ok": true },
                "events": [{
                    "event": "canvas-stream",
                    "payload": { "type": "session_saved", "session_id": "session-1" }
                }]
            })
        );
        assert!(is_canvas_terminal(&event));
        assert!(!is_canvas_terminal(&BufferedEvent {
            event: "canvas-stream".to_string(),
            payload: json!({ "type": "error" }),
        }));
        assert!(!is_canvas_terminal(&BufferedEvent {
            event: "canvas-stream".to_string(),
            payload: json!({ "type": "complete" }),
        }));
        assert!(
            encode_success_body(Value::String("x".repeat(MAX_RESPONSE_BYTES)), vec![]).is_err()
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&encode_error_body("invoke_failed", "Command failed"))
                .unwrap(),
            json!({ "error": { "code": "invoke_failed", "message": "Command failed" } })
        );
    }

    #[tokio::test]
    async fn failed_invocation_drains_buffered_events_before_the_next_command() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let sink = runtime
            .devices
            .get(&TestDevice::DeviceA)
            .unwrap()
            .canvas_events
            .clone();
        crate::commands::canvas::CanvasEventSink::emit_canvas(
            &sink,
            crate::models::canvas::CanvasStreamEvent::Error {
                session_id: "failed-session".into(),
                tile_id: "failed-tile".into(),
                model_id: "failed-model".into(),
                error: "expected failure".into(),
            },
        )
        .unwrap();

        assert!(runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "get_boot_status",
                json!({ "unexpected": true }),
            )
            .await
            .is_err());
        let (_, next_events) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "list_notes",
                json!({}),
            )
            .await
            .unwrap();

        assert!(next_events.is_empty());
    }

    #[tokio::test]
    async fn canvas_terminal_timeout_quarantines_device_from_later_invoke_and_restart() {
        let owned = tempfile::tempdir().unwrap();
        let (openrouter_url, release_openrouter, openrouter_stub) =
            spawn_hanging_openrouter_stub().await;
        let mut runtime =
            TestRuntimeHarness::start(config_with_openrouter(owned.path(), &openrouter_url))
                .await
                .unwrap();
        let session = runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "create_session",
                json!({ "session": { "title": "Terminal timeout quarantine" } }),
            )
            .await
            .unwrap();

        let timeout_error = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "send_prompt",
                json!({
                    "sessionId": session["id"],
                    "request": {
                        "prompt": "Never reaches a terminal event",
                        "models": ["openai/e2e-hanging"]
                    }
                }),
            )
            .await
            .unwrap_err();
        assert!(
            timeout_error.contains("E2E Canvas stream did not reach a terminal event"),
            "{timeout_error}"
        );

        release_openrouter.send(()).unwrap();
        openrouter_stub.await.unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;

        let later_restart = runtime.restart(TestDevice::DeviceA).await;
        let later_rebuild = runtime.rebuild_device(TestDevice::DeviceA).await;
        let later_invoke = runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "list_notes",
                json!({}),
            )
            .await;
        let other_device = runtime
            .invoke_selected(
                TestDevice::DeviceB,
                TestProfile::Desktop,
                "list_notes",
                json!({}),
            )
            .await;
        assert!(later_invoke.is_err());
        assert!(later_restart.is_err());
        assert!(later_rebuild.is_err());
        assert!(!runtime.devices.contains_key(&TestDevice::DeviceA));
        assert!(other_device.is_ok());
    }

    #[tokio::test]
    async fn canvas_error_waits_for_persistence_and_cannot_leak_later_model_events() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let sink = runtime
            .devices
            .get(&TestDevice::DeviceA)
            .unwrap()
            .canvas_events
            .clone();
        crate::commands::canvas::CanvasEventSink::emit_canvas(
            &sink,
            crate::models::canvas::CanvasStreamEvent::Error {
                session_id: "session-multi".into(),
                tile_id: "tile-multi".into(),
                model_id: "model-a".into(),
                error: "model A failed".into(),
            },
        )
        .unwrap();
        let later_sink = sink.clone();
        let later_events = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            crate::commands::canvas::CanvasEventSink::emit_canvas(
                &later_sink,
                crate::models::canvas::CanvasStreamEvent::Complete {
                    session_id: "session-multi".into(),
                    tile_id: "tile-multi".into(),
                    model_id: "model-b".into(),
                    tokens_used: Some(3),
                    cost_usd: None,
                },
            )
            .unwrap();
            crate::commands::canvas::CanvasEventSink::emit_canvas(
                &later_sink,
                crate::models::canvas::CanvasStreamEvent::SessionSaved {
                    session_id: "session-multi".into(),
                },
            )
            .unwrap();
        });

        let events = runtime
            .take_response_events(TestDevice::DeviceA, true)
            .await
            .unwrap();
        later_events.await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.payload.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            ["error", "complete", "session_saved"]
        );
        let (_, next_events) = runtime
            .invoke_for_response(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "list_notes",
                json!({}),
            )
            .await
            .unwrap();
        assert!(next_events.is_empty());
    }

    #[tokio::test]
    async fn devices_are_isolated_but_real_encrypted_sync_converges_without_echo() {
        let owned = tempfile::tempdir().unwrap();
        let mut runtime = TestRuntimeHarness::start(config(owned.path()))
            .await
            .unwrap();
        let created = runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "create_note",
                json!({ "note": { "title": "Device A", "content": "Encrypted handoff" } }),
            )
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let before = runtime
            .invoke_selected(
                TestDevice::DeviceB,
                TestProfile::Desktop,
                "list_notes",
                json!({}),
            )
            .await
            .unwrap();
        assert!(before.as_array().unwrap().is_empty());

        let mut bundle = runtime
            .invoke_selected(
                TestDevice::DeviceA,
                TestProfile::Desktop,
                "export_sync_outbox",
                json!({}),
            )
            .await
            .unwrap();
        let envelopes = bundle["envelopes"].as_array_mut().unwrap();
        envelopes.reverse();
        envelopes.push(envelopes[0].clone());
        let imported = runtime
            .invoke_selected(
                TestDevice::DeviceB,
                TestProfile::Desktop,
                "import_sync_envelopes",
                json!({ "bundle": bundle }),
            )
            .await
            .unwrap();
        assert!(imported["duplicates"].as_u64().unwrap() >= 1);
        let converged = runtime
            .invoke_selected(
                TestDevice::DeviceB,
                TestProfile::Desktop,
                "get_note",
                json!({ "id": id }),
            )
            .await
            .unwrap();
        assert_eq!(converged["content"], "Encrypted handoff");
        let echo = runtime
            .invoke_selected(
                TestDevice::DeviceB,
                TestProfile::Desktop,
                "export_sync_outbox",
                json!({}),
            )
            .await
            .unwrap();
        assert!(echo["envelopes"].as_array().unwrap().is_empty());
    }
}

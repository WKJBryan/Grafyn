use serde::{Deserialize, Serialize};

pub const RUNTIME_STATUS_SCHEMA_VERSION: u16 = 1;
const MAX_RUNTIME_CODE_CHARS: usize = 64;
const MAX_RUNTIME_MESSAGE_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    Desktop,
    Android,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeVaultKind {
    UserSelected,
    AppPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeFeatureAvailability {
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeFeatureStatusV1 {
    pub status: RuntimeFeatureAvailability,
    pub code: Option<String>,
    pub message: Option<String>,
}

impl RuntimeFeatureStatusV1 {
    pub fn ready() -> Self {
        Self {
            status: RuntimeFeatureAvailability::Ready,
            code: None,
            message: None,
        }
    }

    pub fn unavailable(code: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        Self {
            status: RuntimeFeatureAvailability::Unavailable,
            code: Some(sanitize_code(code.as_ref())),
            message: Some(sanitize_message(message.as_ref())),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.status == RuntimeFeatureAvailability::Ready
    }

    pub(crate) fn diagnostic(&self) -> Option<RuntimeDiagnosticV1> {
        match (&self.code, &self.message) {
            (Some(code), Some(message)) if !self.is_ready() => Some(RuntimeDiagnosticV1 {
                code: code.clone(),
                message: message.clone(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilitiesV1 {
    pub notes_read: bool,
    pub notes_write: bool,
    pub recall: bool,
    pub twin_review: bool,
    pub twin_chat: bool,
    pub linear_canvas: bool,
    pub image_generation: bool,
    pub native_image_share: bool,
    pub sync: bool,
    pub spatial_canvas: bool,
    pub native_vault_picker: bool,
    pub import_by_path: bool,
    pub local_ollama: bool,
    pub mcp: bool,
    pub vault_migration: bool,
    pub optimizer_admin: bool,
    pub desktop_updater: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeVaultStatusV1 {
    pub kind: RuntimeVaultKind,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDiagnosticV1 {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStatusV1 {
    pub schema_version: u16,
    pub runtime: RuntimeKind,
    pub capabilities: RuntimeCapabilitiesV1,
    pub vault: RuntimeVaultStatusV1,
    pub secure_secrets: RuntimeFeatureStatusV1,
    pub native_image_share: RuntimeFeatureStatusV1,
    pub diagnostics: Vec<RuntimeDiagnosticV1>,
}

fn sanitize_code(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(MAX_RUNTIME_CODE_CHARS));
    for character in value.chars() {
        if sanitized.chars().count() >= MAX_RUNTIME_CODE_CHARS {
            break;
        }
        let next = if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
            character.to_ascii_lowercase()
        } else {
            '_'
        };
        if next != '_' || !sanitized.ends_with('_') {
            sanitized.push(next);
        }
    }
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "runtime_unavailable".to_string()
    } else {
        sanitized.to_string()
    }
}

fn sanitize_message(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(MAX_RUNTIME_MESSAGE_CHARS));
    let mut pending_space = false;
    for character in value.chars() {
        if sanitized.chars().count() >= MAX_RUNTIME_MESSAGE_CHARS {
            break;
        }
        if character.is_control() || character.is_whitespace() {
            pending_space = !sanitized.is_empty();
            continue;
        }
        if pending_space {
            sanitized.push(' ');
            pending_space = false;
        }
        sanitized.push(character);
    }
    let sanitized = sanitized.trim();
    if sanitized.is_empty() {
        "This capability is unavailable.".to_string()
    } else {
        sanitized.to_string()
    }
}

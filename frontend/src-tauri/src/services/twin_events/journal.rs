use crate::models::twin_event::{
    ActorId, CausalStream, ContentDigest, DeviceId, SourceChannel, TwinEvent,
};
use crate::services::twin_events::{derive_event_id, MutationError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub const MAX_PENDING_INTENTS: usize = 256;
pub const MAX_INTENT_TARGETS: usize = 64;
pub const MAX_INTENT_EVENTS: usize = 64;
pub const MAX_TARGET_KEY_BYTES: usize = 512;
pub const MAX_MARKDOWN_TWIN_BYTES: usize = 1024 * 1024;
pub const MAX_CANVAS_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_AFTER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_EVENT_GROUP_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SERIALIZED_INTENT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetKind {
    Markdown,
    OverlayJson,
    TwinJson,
    CanvasJson,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "digest",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum BeforeImage {
    Absent,
    Sha256(ContentDigest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "content",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DesiredImage {
    Tombstone,
    Utf8Bytes(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutationTargetV1 {
    pub kind: TargetKind,
    pub relative_key: String,
    pub before: BeforeImage,
    pub after: DesiredImage,
    pub after_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetMutation {
    pub kind: TargetKind,
    pub relative_key: String,
    pub after: DesiredImage,
    pub expected_before: Option<BeforeImage>,
    pub(crate) retain_exact_precondition: bool,
    pub(crate) check_expected_before_before_after_elision: bool,
}

impl TargetMutation {
    pub fn put(
        kind: TargetKind,
        relative_key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            relative_key: relative_key.into(),
            after: DesiredImage::Utf8Bytes(content.into()),
            expected_before: None,
            retain_exact_precondition: false,
            check_expected_before_before_after_elision: false,
        }
    }

    pub fn tombstone(kind: TargetKind, relative_key: impl Into<String>) -> Self {
        Self {
            kind,
            relative_key: relative_key.into(),
            after: DesiredImage::Tombstone,
            expected_before: None,
            retain_exact_precondition: false,
            check_expected_before_before_after_elision: false,
        }
    }

    pub fn expecting(mut self, expected_before: BeforeImage) -> Self {
        self.expected_before = Some(expected_before);
        self
    }

    pub(crate) fn retaining_exact_precondition(mut self) -> Self {
        self.retain_exact_precondition = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn checking_expected_before_before_after_elision(mut self) -> Self {
        self.check_expected_before_before_after_elision = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutationIntentV1 {
    pub schema_version: u16,
    pub mutation_id: ContentDigest,
    pub origin: crate::services::twin_events::MutationOrigin,
    pub actor_id: ActorId,
    pub device_id: DeviceId,
    pub causal_stream: CausalStream,
    pub source_channel: SourceChannel,
    #[serde(deserialize_with = "required_option")]
    pub markdown_root_scope: Option<ContentDigest>,
    #[serde(default = "missing_content_authority_generation")]
    pub content_authority_generation: Option<u64>,
    #[serde(default)]
    pub retain_commit_receipt: bool,
    pub targets: Vec<MutationTargetV1>,
    pub events: Vec<TwinEvent>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl MutationIntentV1 {
    pub fn validate(&self) -> Result<(), MutationError> {
        if !matches!(self.schema_version, 1..=3) {
            return Err(MutationError::Invalid(
                "unsupported local mutation journal schema".into(),
            ));
        }
        if self.schema_version < 3 && self.retain_commit_receipt {
            return Err(MutationError::Invalid(
                "legacy mutation intents cannot retain commit receipts".into(),
            ));
        }
        if self.origin != crate::services::twin_events::MutationOrigin::Local
            && !self.events.is_empty()
        {
            return Err(MutationError::Invalid(
                "nonlocal mutation intents cannot contain local events".into(),
            ));
        }
        if self.targets.len() > MAX_INTENT_TARGETS {
            return Err(MutationError::Invalid(
                "mutation intent must contain at most 64 targets".into(),
            ));
        }
        if self.events.len() > MAX_INTENT_EVENTS {
            return Err(MutationError::Invalid(
                "mutation intent must contain at most 64 events".into(),
            ));
        }
        if self.targets.is_empty() && self.events.is_empty() {
            return Err(MutationError::Invalid(
                "mutation intent must contain a target or event".into(),
            ));
        }
        let is_exact_precondition = |target: &MutationTargetV1| {
            self.schema_version == 3
                && self.retain_commit_receipt
                && matches!(
                    (&target.before, &target.after),
                    (BeforeImage::Sha256(before), DesiredImage::Utf8Bytes(_))
                        if before == &target.after_digest
                )
        };
        let has_exact_precondition = self.targets.iter().any(is_exact_precondition);
        if has_exact_precondition && self.targets.iter().all(is_exact_precondition) {
            return Err(MutationError::Invalid(
                "retained exact preconditions require a writable target".into(),
            ));
        }
        let has_vault_scoped_target = self
            .targets
            .iter()
            .any(|target| matches!(target.kind, TargetKind::Markdown | TargetKind::OverlayJson));
        let retained_authority_owner =
            self.retain_commit_receipt && self.content_authority_generation.is_some();
        if self.markdown_root_scope.is_some()
            != (has_vault_scoped_target || retained_authority_owner)
        {
            return Err(MutationError::Invalid(
                "vault-scoped mutations must carry exactly one root scope digest".into(),
            ));
        }
        let changes_authority = !self.events.is_empty()
            || self.targets.iter().any(|target| {
                matches!(
                    target.kind,
                    TargetKind::Markdown | TargetKind::OverlayJson | TargetKind::TwinJson
                )
            });
        match self.schema_version {
            1 if self.content_authority_generation == Some(u64::MAX) => {}
            1 if self.content_authority_generation.is_none() => {}
            1 => {
                return Err(MutationError::Invalid(
                    "legacy mutation intent cannot carry content authority generation".into(),
                ));
            }
            2 | 3
                if changes_authority
                    && self
                        .content_authority_generation
                        .is_some_and(|generation| generation != u64::MAX) => {}
            2 | 3 if !changes_authority && self.content_authority_generation.is_none() => {}
            2 | 3 => {
                return Err(MutationError::Invalid(
                    "mutation intent content authority generation is invalid".into(),
                ));
            }
            _ => unreachable!(),
        }
        let mut target_order = self
            .targets
            .iter()
            .map(|target| (target.kind, target.relative_key.as_str()))
            .collect::<Vec<_>>();
        let original_order = target_order.clone();
        target_order.sort();
        target_order.dedup();
        if target_order != original_order || target_order.len() != self.targets.len() {
            return Err(MutationError::Invalid(
                "mutation targets must be unique and canonically ordered".into(),
            ));
        }
        let mut physical_targets = self
            .targets
            .iter()
            .map(|target| physical_target_key(target.kind, &target.relative_key))
            .collect::<Result<Vec<_>, _>>()?;
        physical_targets.sort();
        let before_dedup = physical_targets.len();
        physical_targets.dedup();
        if physical_targets.len() != before_dedup {
            return Err(MutationError::Invalid(
                "mutation targets contain physical path aliases".into(),
            ));
        }
        let mut total_after = 0usize;
        for target in &self.targets {
            validate_target_key(target.kind, &target.relative_key)?;
            let expected = desired_digest(&target.after);
            if target.after_digest != expected {
                return Err(MutationError::Invalid(
                    "mutation target after digest is invalid".into(),
                ));
            }
            if let DesiredImage::Utf8Bytes(content) = &target.after {
                let length = content.len();
                let limit = match target.kind {
                    TargetKind::Markdown | TargetKind::OverlayJson | TargetKind::TwinJson => {
                        MAX_MARKDOWN_TWIN_BYTES
                    }
                    TargetKind::CanvasJson => MAX_CANVAS_BYTES,
                };
                if length > limit {
                    return Err(MutationError::Invalid(format!(
                        "mutation target exceeds its {limit}-byte limit"
                    )));
                }
                total_after = total_after
                    .checked_add(length)
                    .ok_or_else(|| MutationError::Invalid("mutation size overflow".into()))?;
            }
        }
        if total_after > MAX_TOTAL_AFTER_BYTES {
            return Err(MutationError::Invalid(
                "mutation after-images exceed the 16 MiB total limit".into(),
            ));
        }
        let mut event_bytes = 0usize;
        for event in &self.events {
            event.validate().map_err(MutationError::Invalid)?;
            if derive_event_id(event) != event.event_id {
                return Err(MutationError::Invalid(
                    "mutation intent contains a noncanonical event ID".into(),
                ));
            }
            if event.device_id != self.device_id
                || event.causal_stream != self.causal_stream
                || event.context.source_channel != self.source_channel
            {
                return Err(MutationError::Invalid(
                    "mutation event group identity, stream, or source mismatch".into(),
                ));
            }
            let length = serde_json::to_vec(event)
                .map_err(|error| MutationError::Invalid(error.to_string()))?
                .len();
            if length > crate::services::twin_events::MAX_TWIN_EVENT_BYTES {
                return Err(MutationError::Invalid(
                    "mutation event exceeds the 256 KiB limit".into(),
                ));
            }
            event_bytes = event_bytes
                .checked_add(length)
                .ok_or_else(|| MutationError::Invalid("event group size overflow".into()))?;
        }
        if event_bytes > MAX_EVENT_GROUP_BYTES {
            return Err(MutationError::Invalid(
                "mutation event group exceeds the 4 MiB limit".into(),
            ));
        }
        if derive_mutation_id(self) != self.mutation_id {
            return Err(MutationError::Invalid(
                "mutation ID does not match canonical content".into(),
            ));
        }
        Ok(())
    }
}

pub fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    let digest = Sha256::digest(bytes);
    ContentDigest::parse(format!("{digest:x}")).expect("SHA-256 is a valid content digest")
}

pub fn desired_digest(after: &DesiredImage) -> ContentDigest {
    match after {
        DesiredImage::Tombstone => digest_bytes(b"grafyn.local_mutation.tombstone.v1"),
        DesiredImage::Utf8Bytes(content) => digest_bytes(content.as_bytes()),
    }
}

pub fn derive_mutation_id(intent: &MutationIntentV1) -> ContentDigest {
    fn frame(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let mut hasher = Sha256::new();
    frame(&mut hasher, b"grafyn.local_mutation.v1");
    frame(
        &mut hasher,
        match intent.origin {
            crate::services::twin_events::MutationOrigin::Local => b"local",
            crate::services::twin_events::MutationOrigin::Remote => b"remote",
            crate::services::twin_events::MutationOrigin::Recovery => b"recovery",
        },
    );
    frame(&mut hasher, intent.actor_id.as_str().as_bytes());
    frame(&mut hasher, intent.device_id.as_str().as_bytes());
    frame(
        &mut hasher,
        match intent.causal_stream {
            CausalStream::LocalOnly => b"local_only",
            CausalStream::SyncEligible => b"sync_eligible",
        },
    );
    frame(&mut hasher, intent.source_channel.as_str().as_bytes());
    match &intent.markdown_root_scope {
        Some(scope) => {
            frame(&mut hasher, b"markdown_root_scope");
            frame(&mut hasher, scope.as_str().as_bytes());
        }
        None => frame(&mut hasher, b"no_markdown_root_scope"),
    }
    if intent.schema_version >= 2 {
        match intent.content_authority_generation {
            Some(generation) if generation != u64::MAX => {
                frame(&mut hasher, b"content_authority_generation");
                frame(&mut hasher, &generation.to_be_bytes());
            }
            Some(_) => frame(&mut hasher, b"missing_content_authority_generation"),
            None => frame(&mut hasher, b"no_content_authority_generation"),
        }
    }
    if intent.schema_version >= 3 {
        frame(
            &mut hasher,
            if intent.retain_commit_receipt {
                b"retain_commit_receipt"
            } else {
                b"discard_commit_receipt"
            },
        );
    }
    for event in &intent.events {
        frame(&mut hasher, event.event_id.as_str().as_bytes());
    }
    for target in &intent.targets {
        frame(
            &mut hasher,
            match target.kind {
                TargetKind::Markdown => b"markdown",
                TargetKind::OverlayJson => b"overlay_json",
                TargetKind::TwinJson => b"twin_json",
                TargetKind::CanvasJson => b"canvas_json",
            },
        );
        frame(&mut hasher, target.relative_key.as_bytes());
        match &target.before {
            BeforeImage::Absent => frame(&mut hasher, b"absent"),
            BeforeImage::Sha256(digest) => frame(&mut hasher, digest.as_str().as_bytes()),
        }
        frame(&mut hasher, target.after_digest.as_str().as_bytes());
    }
    ContentDigest::parse(format!("{:x}", hasher.finalize()))
        .expect("SHA-256 is a valid mutation ID")
}

fn missing_content_authority_generation() -> Option<u64> {
    Some(u64::MAX)
}

pub fn validate_relative_key(key: &str) -> Result<(), MutationError> {
    if key.is_empty()
        || key.len() > MAX_TARGET_KEY_BYTES
        || key.contains('\\')
        || key.contains("//")
        || key.as_bytes().get(1) == Some(&b':')
        || key.starts_with('/')
    {
        return Err(MutationError::Invalid(
            "mutation target key must be 1..=512 safe UTF-8 bytes".into(),
        ));
    }
    for component in key.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with('.')
            || component.ends_with(' ')
            || component.contains(':')
            || is_windows_reserved_component(component)
        {
            return Err(MutationError::Invalid(
                "mutation target key contains a physical path alias".into(),
            ));
        }
    }
    Ok(())
}

pub fn physical_target_key(kind: TargetKind, key: &str) -> Result<String, MutationError> {
    validate_target_key(kind, key)?;
    let prefix = match kind {
        TargetKind::Markdown => "markdown",
        TargetKind::OverlayJson => "overlay_json",
        TargetKind::TwinJson => "twin_json",
        TargetKind::CanvasJson => "canvas_json",
    };
    Ok(format!("{prefix}:{}", key.to_lowercase()))
}

pub fn validate_target_key(kind: TargetKind, key: &str) -> Result<(), MutationError> {
    validate_relative_key(key)?;
    if kind == TargetKind::OverlayJson
        && (key.contains('/')
            || key.contains('\\')
            || !key.ends_with(".json")
            || key.len() == ".json".len())
    {
        return Err(MutationError::Invalid(
            "overlay target must be one canonical note-ID JSON leaf".into(),
        ));
    }
    Ok(())
}

fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

const PENDING_DIRECTORY: &str = "twin/mutations/pending/v1";
const PREAUTHORITY_DIRECTORY: &str = "twin/mutations/preauthority/v1";
const QUARANTINE_DIRECTORY: &str = "twin/mutations/quarantine/v1";
const STAGING_DIRECTORY: &str = "twin/mutations/staging/v1";
const RECEIPTS_DIRECTORY: &str = "twin/mutations/receipts/v1";
const MAX_COMMIT_RECEIPTS: usize = 256;
const MAX_COMMIT_RECEIPT_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MutationCommitReceiptV1 {
    pub(crate) schema_version: u16,
    pub(crate) mutation_id: ContentDigest,
    pub(crate) root_scope: ContentDigest,
    pub(crate) lease_epoch_uuid: String,
    pub(crate) authority_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreAuthorityMutationV1 {
    pub(crate) schema_version: u16,
    pub(crate) state: PreAuthorityMutationStateV1,
    pub(crate) expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    pub(crate) intent: MutationIntentV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PreAuthorityMutationStateV1 {
    Prepared,
    AbortedBeforeAuthority,
    AbortedAfterAuthority,
}

impl PreAuthorityMutationV1 {
    fn validate(&self) -> Result<(), MutationError> {
        self.intent.validate()?;
        let intended_generation = self
            .expected_authority
            .authority_generation
            .checked_add(1)
            .ok_or_else(|| {
                MutationError::RecoveryConflict("authority-generation-exhausted".into())
            })?;
        let supported_intent = matches!(
            (
                self.intent.schema_version,
                self.intent.retain_commit_receipt
            ),
            (2, false) | (3, true)
        );
        let scope_matches = self
            .intent
            .markdown_root_scope
            .as_ref()
            .map_or(self.intent.schema_version == 2, |scope| {
                scope == &self.expected_authority.root_scope
            });
        if self.schema_version != 1
            || !supported_intent
            || self.intent.origin != crate::services::twin_events::MutationOrigin::Local
            || self.intent.content_authority_generation != Some(intended_generation)
            || !scope_matches
            || Uuid::parse_str(&self.expected_authority.lease_epoch_uuid)
                .ok()
                .is_none_or(|lease| lease.to_string() != self.expected_authority.lease_epoch_uuid)
        {
            return Err(MutationError::Invalid(
                "invalid pre-authority mutation owner".into(),
            ));
        }
        Ok(())
    }
}

pub struct LocalMutationJournal {
    root: crate::services::twin_events::AnchoredRoot,
}

fn serialize_intent(intent: &MutationIntentV1) -> Result<Vec<u8>, MutationError> {
    let mut bytes = serde_json::to_vec_pretty(intent)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SERIALIZED_INTENT_BYTES {
        return Err(MutationError::Invalid(
            "serialized mutation intent exceeds the 32 MiB limit".into(),
        ));
    }
    Ok(bytes)
}

impl LocalMutationJournal {
    pub fn initialize(data_path: impl AsRef<Path>) -> Result<Self, MutationError> {
        let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
        root.open_directory(PENDING_DIRECTORY, true)?;
        root.open_directory(PREAUTHORITY_DIRECTORY, true)?;
        root.open_directory(QUARANTINE_DIRECTORY, true)?;
        root.open_directory(STAGING_DIRECTORY, true)?;
        root.open_directory(RECEIPTS_DIRECTORY, true)?;
        Ok(Self { root })
    }

    pub(crate) fn pending_count(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<usize, MutationError> {
        self.pending_paths()?
            .len()
            .checked_add(self.preauthority_paths()?.len())
            .ok_or_else(|| MutationError::Invalid("mutation journal count overflow".into()))
    }

    pub(crate) fn retained_owner_count(
        &self,
        lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<usize, MutationError> {
        if !lock.covers_data_path(self.root.canonical_path())? {
            return Err(MutationError::Invalid(
                "mutation journal owner query lock belongs to another data root".into(),
            ));
        }
        let pending = self.pending_paths()?.len();
        let preauthority = self.preauthority_paths()?.len();
        let receipts = self.json_names(RECEIPTS_DIRECTORY)?.len();
        pending
            .checked_add(preauthority)
            .and_then(|count| count.checked_add(receipts))
            .ok_or_else(|| MutationError::Invalid("mutation journal owner count overflow".into()))
    }

    pub(crate) fn quarantine_count(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<usize, MutationError> {
        Ok(self.json_names(QUARANTINE_DIRECTORY)?.len())
    }

    pub(crate) fn stage(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        intent: &MutationIntentV1,
    ) -> Result<(), MutationError> {
        intent.validate()?;
        if self.pending_paths()?.len() >= MAX_PENDING_INTENTS {
            return Err(MutationError::Invalid(
                "local mutation journal has reached 256 pending intents".into(),
            ));
        }
        let bytes = serialize_intent(intent)?;
        let path = self.path_for(&intent.mutation_id);
        self.root
            .install_no_clobber(&path, STAGING_DIRECTORY, &bytes)?;
        let existing = self
            .root
            .read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)?
            .ok_or_else(|| MutationError::Invalid("staged mutation intent disappeared".into()))?;
        if existing != bytes {
            return Err(MutationError::Invalid(
                "mutation ID collision in local journal".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn stage_preauthority(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        intent: &MutationIntentV1,
    ) -> Result<(), MutationError> {
        let marker = PreAuthorityMutationV1 {
            schema_version: 1,
            state: PreAuthorityMutationStateV1::Prepared,
            expected_authority: expected_authority.clone(),
            intent: intent.clone(),
        };
        marker.validate()?;
        let path = self.preauthority_path_for(&intent.mutation_id);
        let loaded = self.load_preauthority(_lock)?;
        if loaded.len() >= MAX_PENDING_INTENTS
            && loaded.iter().all(|(existing, _)| existing != &path)
        {
            return Err(MutationError::Invalid(
                "pre-authority mutation records have reached 256 entries".into(),
            ));
        }
        if loaded.iter().any(|(existing, marker)| {
            existing != &path && marker.state == PreAuthorityMutationStateV1::Prepared
        }) {
            return Err(MutationError::RecoveryConflict(
                "another pre-authority mutation must be recovered first".into(),
            ));
        }
        let mut bytes = serde_json::to_vec_pretty(&marker)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_SERIALIZED_INTENT_BYTES {
            return Err(MutationError::Invalid(
                "serialized pre-authority mutation exceeds the 32 MiB limit".into(),
            ));
        }
        self.root
            .install_no_clobber(&path, STAGING_DIRECTORY, &bytes)?;
        let existing = self
            .root
            .read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)?
            .ok_or_else(|| MutationError::Invalid("pre-authority mutation disappeared".into()))?;
        if existing != bytes {
            return Err(MutationError::Invalid(
                "pre-authority mutation identity collision".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn load_preauthority(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<Vec<(String, PreAuthorityMutationV1)>, MutationError> {
        let mut loaded = Vec::new();
        let mut prepared_count = 0usize;
        for path in self.preauthority_paths()? {
            let bytes = self
                .root
                .read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)?
                .ok_or_else(|| {
                    MutationError::Invalid("pre-authority mutation disappeared".into())
                })?;
            let marker: PreAuthorityMutationV1 =
                serde_json::from_slice(&bytes).map_err(|error| {
                    MutationError::Invalid(format!("corrupt pre-authority mutation: {error}"))
                })?;
            marker.validate()?;
            if marker.state == PreAuthorityMutationStateV1::Prepared {
                prepared_count += 1;
                if prepared_count > 1 {
                    return Err(MutationError::Invalid(
                        "local mutation journal has multiple prepared pre-authority owners".into(),
                    ));
                }
            }
            if path != self.preauthority_path_for(&marker.intent.mutation_id) {
                return Err(MutationError::Invalid(
                    "pre-authority mutation is stored at a noncanonical path".into(),
                ));
            }
            loaded.push((path, marker));
        }
        Ok(loaded)
    }

    pub(crate) fn promote_preauthority(
        &self,
        lock: &crate::services::twin_events::CoordinatorProcessLock,
        marker: &PreAuthorityMutationV1,
    ) -> Result<(), MutationError> {
        marker.validate()?;
        if marker.state != PreAuthorityMutationStateV1::Prepared {
            return Err(MutationError::RecoveryConflict(
                "aborted pre-authority mutation cannot be promoted".into(),
            ));
        }
        self.stage(lock, &marker.intent)?;
        let path = self.path_for(&marker.intent.mutation_id);
        let expected = serialize_intent(&marker.intent)?;
        let durable = self
            .root
            .read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)?
            .ok_or_else(|| MutationError::Invalid("promoted mutation WAL disappeared".into()))?;
        if durable != expected {
            return Err(MutationError::Invalid(
                "promoted mutation WAL is not byte-identical".into(),
            ));
        }
        self.root
            .delete(&self.preauthority_path_for(&marker.intent.mutation_id))
    }

    pub(crate) fn abort_preauthority(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        marker: &PreAuthorityMutationV1,
    ) -> Result<(), MutationError> {
        marker.validate()?;
        if marker.state != PreAuthorityMutationStateV1::Prepared {
            return Ok(());
        }
        if !marker.intent.retain_commit_receipt {
            // Schema-2 mutations have no external owner witness to
            // acknowledge an abort tombstone. At the unchanged authority the
            // marker itself proves there was no effect, so retiring it is the
            // complete durable outcome and the caller may replan.
            return self
                .root
                .delete(&self.preauthority_path_for(&marker.intent.mutation_id));
        }
        let aborted = PreAuthorityMutationV1 {
            state: PreAuthorityMutationStateV1::AbortedBeforeAuthority,
            ..marker.clone()
        };
        let mut bytes = serde_json::to_vec_pretty(&aborted)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_SERIALIZED_INTENT_BYTES {
            return Err(MutationError::Invalid(
                "serialized aborted mutation exceeds the 32 MiB limit".into(),
            ));
        }
        let path = self.preauthority_path_for(&marker.intent.mutation_id);
        self.root.put_atomic(&path, &bytes)?;
        let durable = self
            .root
            .read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)?
            .ok_or_else(|| MutationError::Invalid("aborted mutation disappeared".into()))?;
        if durable != bytes {
            return Err(MutationError::Invalid(
                "aborted mutation was not published durably".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn retain_aborted_after_authority(
        &self,
        lock: &crate::services::twin_events::CoordinatorProcessLock,
        intent: &MutationIntentV1,
        committed_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(), MutationError> {
        let expected_generation = committed_authority
            .authority_generation
            .checked_sub(1)
            .ok_or_else(|| {
                MutationError::RecoveryConflict("authority-generation-underflow".into())
            })?;
        let marker = PreAuthorityMutationV1 {
            schema_version: 1,
            state: PreAuthorityMutationStateV1::AbortedAfterAuthority,
            expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1 {
                root_scope: committed_authority.root_scope.clone(),
                lease_epoch_uuid: committed_authority.lease_epoch_uuid.clone(),
                authority_generation: expected_generation,
            },
            intent: intent.clone(),
        };
        marker.validate()?;
        if marker.intent.content_authority_generation
            != Some(committed_authority.authority_generation)
        {
            return Err(MutationError::RecoveryConflict(
                "post-authority abort does not match the committed authority".into(),
            ));
        }
        let path = self.preauthority_path_for(&intent.mutation_id);
        let loaded = self.load_preauthority(lock)?;
        if loaded.len() >= MAX_PENDING_INTENTS
            && loaded.iter().all(|(existing, _)| existing != &path)
        {
            return Err(MutationError::Invalid(
                "pre-authority mutation records have reached 256 entries".into(),
            ));
        }
        let mut bytes = serde_json::to_vec_pretty(&marker)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_SERIALIZED_INTENT_BYTES {
            return Err(MutationError::Invalid(
                "serialized post-authority abort exceeds the 32 MiB limit".into(),
            ));
        }
        if let Some((_, existing)) = loaded.iter().find(|(existing, _)| existing == &path) {
            if existing == &marker {
                return Ok(());
            }
            if existing.state == PreAuthorityMutationStateV1::Prepared
                && existing.expected_authority == marker.expected_authority
                && existing.intent == marker.intent
            {
                self.root.put_atomic(&path, &bytes)?;
            } else {
                return Err(MutationError::RecoveryConflict(
                    "post-authority abort identity collision".into(),
                ));
            }
        } else {
            self.root
                .install_no_clobber(&path, STAGING_DIRECTORY, &bytes)?;
        }
        let durable = self
            .root
            .read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)?
            .ok_or_else(|| MutationError::Invalid("post-authority abort disappeared".into()))?;
        if durable != bytes {
            return Err(MutationError::RecoveryConflict(
                "post-authority abort identity collision".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn preauthority_for(
        &self,
        lock: &crate::services::twin_events::CoordinatorProcessLock,
        mutation_id: &ContentDigest,
    ) -> Result<Option<PreAuthorityMutationV1>, MutationError> {
        Ok(self
            .load_preauthority(lock)?
            .into_iter()
            .find_map(|(_, marker)| (marker.intent.mutation_id == *mutation_id).then_some(marker)))
    }

    pub(crate) fn consume_aborted_preauthority(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        mutation_id: &ContentDigest,
    ) -> Result<(), MutationError> {
        let path = self.preauthority_path_for(mutation_id);
        let Some(bytes) = self.root.read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)? else {
            return Ok(());
        };
        let marker: PreAuthorityMutationV1 = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        marker.validate()?;
        if marker.intent.mutation_id != *mutation_id
            || !matches!(
                marker.state,
                PreAuthorityMutationStateV1::AbortedBeforeAuthority
                    | PreAuthorityMutationStateV1::AbortedAfterAuthority
            )
        {
            return Err(MutationError::RecoveryConflict(
                "only an acknowledged aborted mutation can be consumed".into(),
            ));
        }
        self.root.delete(&path)
    }

    pub(crate) fn remove_matching_aborted_wal(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        marker: &PreAuthorityMutationV1,
    ) -> Result<bool, MutationError> {
        marker.validate()?;
        if marker.state != PreAuthorityMutationStateV1::AbortedAfterAuthority {
            return Err(MutationError::RecoveryConflict(
                "only a post-authority abort can retire a staged WAL".into(),
            ));
        }
        let path = self.path_for(&marker.intent.mutation_id);
        let Some(durable) = self.root.read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES)? else {
            return Ok(false);
        };
        if durable != serialize_intent(&marker.intent)? {
            return Err(MutationError::RecoveryConflict(
                "post-authority abort does not match the staged WAL".into(),
            ));
        }
        self.root.delete(&path)?;
        Ok(true)
    }

    pub(crate) fn load_pending(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<Vec<(String, MutationIntentV1)>, MutationError> {
        let mut loaded = Vec::new();
        for path in self.pending_paths()? {
            let bytes = match self.root.read_bounded(&path, MAX_SERIALIZED_INTENT_BYTES) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    return Err(MutationError::Invalid(
                        "pending mutation intent disappeared".into(),
                    ));
                }
                Err(error) => {
                    self.quarantine_path(&path)?;
                    return Err(error);
                }
            };
            let intent: MutationIntentV1 = match serde_json::from_slice(&bytes) {
                Ok(intent) => intent,
                Err(error) => {
                    self.quarantine_path(&path)?;
                    return Err(MutationError::Invalid(format!(
                        "corrupt mutation intent: {error}"
                    )));
                }
            };
            if let Err(error) = intent.validate() {
                self.quarantine_path(&path)?;
                return Err(error);
            }
            if path != self.path_for(&intent.mutation_id) {
                self.quarantine_path(&path)?;
                return Err(MutationError::Invalid(
                    "mutation intent is stored at a noncanonical path".into(),
                ));
            }
            loaded.push((path, intent));
        }
        Ok(loaded)
    }

    pub(crate) fn remove(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        intent: &MutationIntentV1,
    ) -> Result<(), MutationError> {
        let path = self.path_for(&intent.mutation_id);
        self.root.delete(&path)
    }

    pub(crate) fn retain_committed_receipt(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        intent: &MutationIntentV1,
        authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(), MutationError> {
        if !intent.retain_commit_receipt {
            return Ok(());
        }
        if intent.schema_version != 3
            || intent.markdown_root_scope.as_ref() != Some(&authority.root_scope)
            || intent.content_authority_generation != Some(authority.authority_generation)
            || Uuid::parse_str(&authority.lease_epoch_uuid)
                .ok()
                .is_none_or(|epoch| epoch.to_string() != authority.lease_epoch_uuid)
        {
            return Err(MutationError::Invalid(
                "mutation commit receipt authority does not match its finalized intent".into(),
            ));
        }
        let names = self.json_names(RECEIPTS_DIRECTORY)?;
        let path = self.receipt_path_for(&intent.mutation_id);
        let canonical_name = format!("{}.json", intent.mutation_id.as_str());
        if names.len() >= MAX_COMMIT_RECEIPTS && !names.contains(&canonical_name) {
            return Err(MutationError::Invalid(
                "retained mutation receipts have reached 256 entries".into(),
            ));
        }
        let receipt = MutationCommitReceiptV1 {
            schema_version: 1,
            mutation_id: intent.mutation_id.clone(),
            root_scope: authority.root_scope.clone(),
            lease_epoch_uuid: authority.lease_epoch_uuid.clone(),
            authority_generation: authority.authority_generation,
        };
        let bytes = serde_json::to_vec(&receipt)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        if bytes.len() > MAX_COMMIT_RECEIPT_BYTES {
            return Err(MutationError::Invalid(
                "mutation commit receipt exceeds its 4 KiB limit".into(),
            ));
        }
        self.root
            .install_no_clobber(&path, STAGING_DIRECTORY, &bytes)?;
        let existing = self
            .root
            .read_bounded(&path, MAX_COMMIT_RECEIPT_BYTES)?
            .ok_or_else(|| MutationError::Invalid("mutation commit receipt disappeared".into()))?;
        let existing: MutationCommitReceiptV1 = serde_json::from_slice(&existing)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        if existing == receipt {
            Ok(())
        } else {
            Err(MutationError::Invalid(
                "mutation commit receipt collision".into(),
            ))
        }
    }

    pub(crate) fn preflight_commit_receipt_slot(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        intent: &MutationIntentV1,
        authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(), MutationError> {
        if !intent.retain_commit_receipt {
            return Ok(());
        }
        let names = self.json_names(RECEIPTS_DIRECTORY)?;
        let canonical_name = format!("{}.json", intent.mutation_id.as_str());
        if names.len() >= MAX_COMMIT_RECEIPTS && !names.contains(&canonical_name) {
            return Err(MutationError::Invalid(
                "retained mutation receipts have reached 256 entries".into(),
            ));
        }
        if names.contains(&canonical_name) {
            let receipt = self
                .load_committed_receipt(_lock, &intent.mutation_id)?
                .ok_or_else(|| MutationError::Invalid("mutation receipt disappeared".into()))?;
            if receipt.root_scope != authority.root_scope
                || receipt.lease_epoch_uuid != authority.lease_epoch_uuid
                || receipt.authority_generation != authority.authority_generation
                || receipt.authority_generation
                    != intent.content_authority_generation.unwrap_or_default()
            {
                return Err(MutationError::Invalid(
                    "retained mutation receipt identity collision".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn load_committed_receipt(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        mutation_id: &ContentDigest,
    ) -> Result<Option<MutationCommitReceiptV1>, MutationError> {
        let Some(bytes) = self.root.read_bounded(
            &self.receipt_path_for(mutation_id),
            MAX_COMMIT_RECEIPT_BYTES,
        )?
        else {
            return Ok(None);
        };
        let receipt: MutationCommitReceiptV1 = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        if receipt.schema_version != 1 || receipt.mutation_id != *mutation_id {
            return Err(MutationError::Invalid(
                "invalid retained mutation commit receipt".into(),
            ));
        }
        if Uuid::parse_str(&receipt.lease_epoch_uuid)
            .ok()
            .is_none_or(|epoch| epoch.to_string() != receipt.lease_epoch_uuid)
        {
            return Err(MutationError::Invalid(
                "invalid retained mutation commit receipt lease".into(),
            ));
        }
        Ok(Some(receipt))
    }

    pub(crate) fn consume_committed_receipt(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        mutation_id: &ContentDigest,
    ) -> Result<(), MutationError> {
        let receipt = self.load_committed_receipt(_lock, mutation_id)?;
        let marker = self.preauthority_for(_lock, mutation_id)?;
        match (receipt, marker) {
            (Some(_), Some(_)) => Err(MutationError::RecoveryConflict(
                "mutation has conflicting committed and pre-authority proofs".into(),
            )),
            (None, Some(marker)) if marker.state == PreAuthorityMutationStateV1::Prepared => {
                Err(MutationError::RecoveryConflict(
                    "active pre-authority owner cannot be consumed".into(),
                ))
            }
            (Some(_), None) => self.root.delete(&self.receipt_path_for(mutation_id)),
            (None, Some(_)) => self.consume_aborted_preauthority(_lock, mutation_id),
            (None, None) => Ok(()),
        }
    }

    pub(crate) fn quarantine_intent(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
        intent: &MutationIntentV1,
    ) -> Result<(), MutationError> {
        self.quarantine_path(&self.path_for(&intent.mutation_id))
    }

    fn path_for(&self, id: &ContentDigest) -> String {
        format!("{PENDING_DIRECTORY}/{}.json", id.as_str())
    }

    fn receipt_path_for(&self, id: &ContentDigest) -> String {
        format!("{RECEIPTS_DIRECTORY}/{}.json", id.as_str())
    }

    fn preauthority_path_for(&self, id: &ContentDigest) -> String {
        format!("{PREAUTHORITY_DIRECTORY}/{}.json", id.as_str())
    }

    fn pending_paths(&self) -> Result<Vec<String>, MutationError> {
        let mut paths = self
            .json_names(PENDING_DIRECTORY)?
            .into_iter()
            .map(|name| format!("{PENDING_DIRECTORY}/{name}"))
            .collect::<Vec<_>>();
        if paths.len() > MAX_PENDING_INTENTS {
            return Err(MutationError::Invalid(
                "local mutation journal exceeds 256 pending intents".into(),
            ));
        }
        paths.sort();
        Ok(paths)
    }

    fn preauthority_paths(&self) -> Result<Vec<String>, MutationError> {
        let mut paths = self
            .json_names(PREAUTHORITY_DIRECTORY)?
            .into_iter()
            .map(|name| format!("{PREAUTHORITY_DIRECTORY}/{name}"))
            .collect::<Vec<_>>();
        if paths.len() > MAX_PENDING_INTENTS {
            return Err(MutationError::Invalid(
                "local mutation journal exceeds 256 pre-authority records".into(),
            ));
        }
        paths.sort();
        Ok(paths)
    }

    fn quarantine_path(&self, source: &str) -> Result<(), MutationError> {
        if self.json_names(QUARANTINE_DIRECTORY)?.len() >= MAX_PENDING_INTENTS {
            return Err(MutationError::Invalid(
                "mutation quarantine has reached 256 entries".into(),
            ));
        }
        let name = source.rsplit('/').next().unwrap_or("intent.json");
        let target = format!("{QUARANTINE_DIRECTORY}/{}-{name}", Uuid::new_v4());
        self.root.rename(source, &target, false)
    }

    pub(crate) fn cleanup_orphan_temps_locked(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<(), MutationError> {
        for directory in [
            PENDING_DIRECTORY,
            PREAUTHORITY_DIRECTORY,
            QUARANTINE_DIRECTORY,
            STAGING_DIRECTORY,
            RECEIPTS_DIRECTORY,
        ] {
            let names = self
                .root
                .regular_file_names_bounded(directory, directory_entry_limit(directory))?;
            if let Some(name) = names.iter().find(|name| {
                !(name.starts_with('.') && name.ends_with(".tmp"))
                    && (directory == STAGING_DIRECTORY || !name.ends_with(".json"))
            }) {
                return Err(MutationError::Invalid(format!(
                    "mutation journal contains an unexpected entry: {name}"
                )));
            }
            for name in names {
                if name.starts_with('.') && name.ends_with(".tmp") {
                    self.root.delete(&format!("{directory}/{name}"))?;
                }
            }
        }
        Ok(())
    }

    fn json_names(&self, directory: &str) -> Result<Vec<String>, MutationError> {
        let names = self
            .root
            .regular_file_names_bounded(directory, directory_entry_limit(directory))?;
        if let Some(name) = names.iter().find(|name| !name.ends_with(".json")) {
            return Err(MutationError::Invalid(format!(
                "mutation journal contains a non-JSON entry: {name}"
            )));
        }
        Ok(names)
    }
}

fn directory_entry_limit(directory: &str) -> usize {
    if directory == RECEIPTS_DIRECTORY {
        MAX_COMMIT_RECEIPTS
    } else {
        MAX_PENDING_INTENTS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::{CausalStream, SourceChannel};
    use crate::services::twin_events::{
        MutationCoordinator, MutationFaultPoint, NoopMutationLifecycle, TargetMutation,
        TwinEventStore,
    };
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn draft(label: &str) -> crate::services::twin_events::TwinEventDraft {
        use crate::models::twin_event::{
            Governance, Identifier, NoteChangeKind, NoteChanged, TwinEventPayload,
        };
        use chrono::{TimeZone, Utc};
        crate::services::twin_events::TwinEventDraft {
            actor_id: None,
            causal_parents: Vec::new(),
            recorded_at: Utc.with_ymd_and_hms(2026, 8, 30, 2, 0, 0).unwrap(),
            observed_at: Utc.with_ymd_and_hms(2026, 8, 30, 2, 0, 0).unwrap(),
            occurred_at: None,
            valid_from: None,
            valid_to: None,
            supersedes: Vec::new(),
            reinforces: Vec::new(),
            context: Default::default(),
            evidence: Vec::new(),
            governance: Governance::direct_observation(),
            payload: TwinEventPayload::NoteChanged(NoteChanged {
                note_id: Identifier::parse(label).unwrap(),
                change: NoteChangeKind::Updated,
                content_digest: None,
            }),
        }
    }

    fn legacy_markdown_intent(
        root_scope: ContentDigest,
        relative_key: &str,
        before: BeforeImage,
        after: &str,
    ) -> MutationIntentV1 {
        let mut intent = MutationIntentV1 {
            schema_version: 1,
            mutation_id: digest_bytes(b"placeholder"),
            origin: crate::services::twin_events::MutationOrigin::Local,
            actor_id: crate::models::twin_event::ActorId::parse("owner").unwrap(),
            device_id: crate::models::twin_event::DeviceId::parse("device").unwrap(),
            causal_stream: CausalStream::LocalOnly,
            source_channel: SourceChannel::parse("note_editor").unwrap(),
            markdown_root_scope: Some(root_scope),
            content_authority_generation: None,
            retain_commit_receipt: false,
            targets: vec![MutationTargetV1 {
                kind: TargetKind::Markdown,
                relative_key: relative_key.to_string(),
                before,
                after: DesiredImage::Utf8Bytes(after.to_string()),
                after_digest: digest_bytes(after.as_bytes()),
            }],
            events: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        intent.mutation_id = derive_mutation_id(&intent);
        intent.validate().unwrap();
        intent
    }

    fn stage_manual_intent(
        data_path: &Path,
        coordinator: &MutationCoordinator,
        intent: &MutationIntentV1,
    ) {
        let guard = coordinator.begin_root_transition().unwrap();
        LocalMutationJournal::initialize(data_path)
            .unwrap()
            .stage(guard.process_lock(), intent)
            .unwrap();
    }

    #[test]
    fn schema_two_authority_generation_is_required_and_changes_mutation_identity() {
        let mut intent = MutationIntentV1 {
            schema_version: 2,
            mutation_id: digest_bytes(b"placeholder"),
            origin: crate::services::twin_events::MutationOrigin::Local,
            actor_id: crate::models::twin_event::ActorId::parse("owner").unwrap(),
            device_id: crate::models::twin_event::DeviceId::parse("device").unwrap(),
            causal_stream: CausalStream::LocalOnly,
            source_channel: SourceChannel::parse("note_editor").unwrap(),
            markdown_root_scope: Some(digest_bytes(b"root")),
            content_authority_generation: Some(7),
            retain_commit_receipt: false,
            targets: vec![MutationTargetV1 {
                kind: TargetKind::Markdown,
                relative_key: "generation.md".to_string(),
                before: BeforeImage::Absent,
                after: DesiredImage::Utf8Bytes("after".to_string()),
                after_digest: digest_bytes(b"after"),
            }],
            events: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        intent.mutation_id = derive_mutation_id(&intent);
        intent.validate().unwrap();
        let original_id = intent.mutation_id.clone();
        let bytes = serde_json::to_vec(&intent).unwrap();

        let mut next = intent.clone();
        next.content_authority_generation = Some(
            intent
                .content_authority_generation
                .unwrap()
                .checked_add(1)
                .unwrap(),
        );
        next.mutation_id = derive_mutation_id(&next);
        next.validate().unwrap();
        assert_ne!(next.mutation_id, original_id);

        let mut missing: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        missing
            .as_object_mut()
            .unwrap()
            .remove("content_authority_generation");
        let missing: MutationIntentV1 = serde_json::from_value(missing).unwrap();
        assert!(missing.validate().is_err());

        let mut legacy_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let object = legacy_value.as_object_mut().unwrap();
        object.insert("schema_version".into(), serde_json::json!(1));
        object.remove("content_authority_generation");
        let mut legacy: MutationIntentV1 = serde_json::from_value(legacy_value).unwrap();
        legacy.mutation_id = derive_mutation_id(&legacy);
        legacy.validate().unwrap();
        assert_eq!(legacy.content_authority_generation, Some(u64::MAX));

        let mut invalid_legacy_receipt = legacy.clone();
        invalid_legacy_receipt.retain_commit_receipt = true;
        invalid_legacy_receipt.mutation_id = derive_mutation_id(&invalid_legacy_receipt);
        assert!(invalid_legacy_receipt.validate().is_err());

        let mut schema_three = intent.clone();
        schema_three.schema_version = 3;
        schema_three.retain_commit_receipt = true;
        schema_three.mutation_id = derive_mutation_id(&schema_three);
        schema_three.validate().unwrap();
        assert_ne!(schema_three.mutation_id, original_id);

        let mut guard_only = schema_three.clone();
        let after = guard_only.targets[0].after_digest.clone();
        guard_only.targets[0].before = BeforeImage::Sha256(after);
        guard_only.mutation_id = derive_mutation_id(&guard_only);
        let guard_only: MutationIntentV1 =
            serde_json::from_slice(&serde_json::to_vec(&guard_only).unwrap()).unwrap();
        assert!(guard_only
            .validate()
            .unwrap_err()
            .to_string()
            .contains("writable target"));

        // Schema-1/2 canonical IDs are unchanged by the newly deserializable
        // default-false receipt field.
        let mut legacy_roundtrip: serde_json::Value = serde_json::to_value(&legacy).unwrap();
        legacy_roundtrip
            .as_object_mut()
            .unwrap()
            .remove("retain_commit_receipt");
        let legacy_roundtrip: MutationIntentV1 = serde_json::from_value(legacy_roundtrip).unwrap();
        assert_eq!(derive_mutation_id(&legacy_roundtrip), legacy.mutation_id);
        let mut v2_roundtrip: serde_json::Value = serde_json::to_value(&intent).unwrap();
        v2_roundtrip
            .as_object_mut()
            .unwrap()
            .remove("retain_commit_receipt");
        let v2_roundtrip: MutationIntentV1 = serde_json::from_value(v2_roundtrip).unwrap();
        assert_eq!(derive_mutation_id(&v2_roundtrip), intent.mutation_id);
    }

    #[test]
    fn retained_receipts_preflight_capacity_and_bind_exact_authority() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        let current = guard.capture_authority_token(&lease).unwrap();
        let committed = crate::services::vault_namespace::VaultAuthorityTokenV1 {
            root_scope: current.root_scope.clone(),
            lease_epoch_uuid: current.lease_epoch_uuid.clone(),
            authority_generation: current.authority_generation + 1,
        };
        let mut intent = MutationIntentV1 {
            schema_version: 3,
            mutation_id: digest_bytes(b"placeholder"),
            origin: crate::services::twin_events::MutationOrigin::Local,
            actor_id: crate::models::twin_event::ActorId::parse("owner").unwrap(),
            device_id: crate::models::twin_event::DeviceId::parse("device").unwrap(),
            causal_stream: CausalStream::LocalOnly,
            source_channel: SourceChannel::parse("vault_optimizer").unwrap(),
            markdown_root_scope: Some(committed.root_scope.clone()),
            content_authority_generation: Some(committed.authority_generation),
            retain_commit_receipt: true,
            targets: vec![MutationTargetV1 {
                kind: TargetKind::Markdown,
                relative_key: "receipt.md".to_string(),
                before: BeforeImage::Absent,
                after: DesiredImage::Utf8Bytes("after".to_string()),
                after_digest: digest_bytes(b"after"),
            }],
            events: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        intent.mutation_id = derive_mutation_id(&intent);
        let journal = LocalMutationJournal::initialize(temp.path()).unwrap();
        journal
            .preflight_commit_receipt_slot(guard.process_lock(), &intent, &committed)
            .unwrap();
        journal
            .retain_committed_receipt(guard.process_lock(), &intent, &committed)
            .unwrap();
        let receipt = journal
            .load_committed_receipt(guard.process_lock(), &intent.mutation_id)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.mutation_id, intent.mutation_id);
        assert_eq!(receipt.root_scope, committed.root_scope);
        assert_eq!(receipt.lease_epoch_uuid, committed.lease_epoch_uuid);
        assert_eq!(receipt.authority_generation, committed.authority_generation);

        let replacement = MutationCommitReceiptV1 {
            root_scope: digest_bytes(b"replacement-root"),
            ..receipt.clone()
        };
        journal
            .root
            .put_atomic(
                &journal.receipt_path_for(&intent.mutation_id),
                &serde_json::to_vec(&replacement).unwrap(),
            )
            .unwrap();
        assert!(journal
            .retain_committed_receipt(guard.process_lock(), &intent, &committed)
            .is_err());

        journal
            .root
            .put_atomic(
                &journal.receipt_path_for(&intent.mutation_id),
                &serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();

        let mut wrong = committed.clone();
        wrong.lease_epoch_uuid = Uuid::new_v4().to_string();
        assert!(journal
            .preflight_commit_receipt_slot(guard.process_lock(), &intent, &wrong)
            .is_err());

        for index in 1..MAX_COMMIT_RECEIPTS {
            let id = digest_bytes(format!("receipt-cap-{index}").as_bytes());
            journal
                .root
                .put_atomic(
                    &journal.receipt_path_for(&id),
                    serde_json::to_vec(&MutationCommitReceiptV1 {
                        schema_version: 1,
                        mutation_id: id,
                        root_scope: committed.root_scope.clone(),
                        lease_epoch_uuid: committed.lease_epoch_uuid.clone(),
                        authority_generation: committed.authority_generation,
                    })
                    .unwrap()
                    .as_slice(),
                )
                .unwrap();
        }

        // A canonical receipt does not exempt the directory from the global
        // cap. The inventory must fail closed before adopting any existing
        // proof when an extra entry is present.
        let overflow_id = digest_bytes(b"receipt-cap-overflow");
        journal
            .root
            .put_atomic(
                &journal.receipt_path_for(&overflow_id),
                serde_json::to_vec(&MutationCommitReceiptV1 {
                    schema_version: 1,
                    mutation_id: overflow_id,
                    root_scope: committed.root_scope.clone(),
                    lease_epoch_uuid: committed.lease_epoch_uuid.clone(),
                    authority_generation: committed.authority_generation,
                })
                .unwrap()
                .as_slice(),
            )
            .unwrap();
        assert!(journal
            .preflight_commit_receipt_slot(guard.process_lock(), &intent, &committed)
            .is_err());
        journal
            .root
            .delete(&journal.receipt_path_for(&digest_bytes(b"receipt-cap-overflow")))
            .unwrap();

        let mut another = intent.clone();
        another.targets[0].relative_key = "another.md".to_string();
        another.mutation_id = derive_mutation_id(&another);
        assert!(journal
            .preflight_commit_receipt_slot(guard.process_lock(), &another, &committed)
            .is_err());

        let removed = digest_bytes(b"receipt-cap-1");
        journal
            .root
            .delete(&journal.receipt_path_for(&removed))
            .unwrap();
        let suffix = another.mutation_id.as_str().chars().next_back().unwrap();
        journal
            .root
            .put_atomic(
                &format!("{RECEIPTS_DIRECTORY}/{suffix}.json"),
                &serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
        assert!(journal
            .retain_committed_receipt(guard.process_lock(), &another, &committed)
            .is_err());
    }

    #[test]
    fn postauthority_abort_rejects_a_nonidentical_staged_wal_before_cleanup() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        let expected = guard.capture_authority_token(&lease).unwrap();
        let committed = crate::services::vault_namespace::VaultAuthorityTokenV1 {
            root_scope: expected.root_scope.clone(),
            lease_epoch_uuid: expected.lease_epoch_uuid.clone(),
            authority_generation: expected.authority_generation + 1,
        };
        let mut intent = MutationIntentV1 {
            schema_version: 3,
            mutation_id: digest_bytes(b"placeholder"),
            origin: crate::services::twin_events::MutationOrigin::Local,
            actor_id: crate::models::twin_event::ActorId::parse("owner").unwrap(),
            device_id: crate::models::twin_event::DeviceId::parse("device").unwrap(),
            causal_stream: CausalStream::LocalOnly,
            source_channel: SourceChannel::parse("vault_optimizer").unwrap(),
            markdown_root_scope: Some(committed.root_scope.clone()),
            content_authority_generation: Some(committed.authority_generation),
            retain_commit_receipt: true,
            targets: vec![MutationTargetV1 {
                kind: TargetKind::Markdown,
                relative_key: "abort.md".to_string(),
                before: BeforeImage::Absent,
                after: DesiredImage::Utf8Bytes("after".to_string()),
                after_digest: digest_bytes(b"after"),
            }],
            events: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        intent.mutation_id = derive_mutation_id(&intent);
        intent.validate().unwrap();
        let journal = LocalMutationJournal::initialize(temp.path()).unwrap();
        journal.stage(guard.process_lock(), &intent).unwrap();
        journal
            .retain_aborted_after_authority(guard.process_lock(), &intent, &committed)
            .unwrap();

        let pending_path = journal.path_for(&intent.mutation_id);
        let mut nonidentical = serialize_intent(&intent).unwrap();
        nonidentical.push(b' ');
        journal
            .root
            .put_atomic(&pending_path, &nonidentical)
            .unwrap();
        let marker = journal
            .preauthority_for(guard.process_lock(), &intent.mutation_id)
            .unwrap()
            .unwrap();
        assert!(journal
            .remove_matching_aborted_wal(guard.process_lock(), &marker)
            .is_err());
        assert_eq!(
            journal
                .root
                .read_bounded(&pending_path, MAX_SERIALIZED_INTENT_BYTES)
                .unwrap()
                .unwrap(),
            nonidentical
        );
        assert_eq!(
            journal
                .preauthority_for(guard.process_lock(), &intent.mutation_id)
                .unwrap()
                .unwrap()
                .state,
            PreAuthorityMutationStateV1::AbortedAfterAuthority
        );
    }

    #[test]
    fn staged_mixed_targets_and_partial_events_recover_idempotently() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        std::fs::write(vault.join("old.md"), "old").unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            temp.path(),
            &vault,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        coordinator.fail_once_at(MutationFaultPoint::AfterTarget(0));
        let commit = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![
                    TargetMutation::put(TargetKind::Markdown, "new.md", "new"),
                    TargetMutation::tombstone(TargetKind::Markdown, "old.md"),
                ],
                vec![draft("move")],
            )
            .expect("the in-call replay must finish an exact partial effect");
        assert!(commit.postcommit_warning);
        assert_eq!(
            std::fs::read_to_string(vault.join("new.md")).unwrap(),
            "new"
        );
        assert!(!vault.join("old.md").exists());
        assert_eq!(store.ordered_events().unwrap().len(), 1);
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert_eq!(store.ordered_events().unwrap().len(), 1);
    }

    #[test]
    fn third_digest_preserves_user_bytes_and_quarantines_intent() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        std::fs::write(vault.join("note.md"), "before").unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let scope = coordinator.current_authority_token().unwrap().root_scope;
        let intent = legacy_markdown_intent(
            scope,
            "note.md",
            BeforeImage::Sha256(digest_bytes(b"before")),
            "desired",
        );
        stage_manual_intent(temp.path(), &coordinator, &intent);
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        std::fs::write(vault.join("note.md"), "unexpected newer bytes").unwrap();
        assert!(matches!(
            coordinator.recover_pending(),
            Err(crate::services::twin_events::MutationError::RecoveryConflict(_))
        ));
        assert_eq!(
            std::fs::read_to_string(vault.join("note.md")).unwrap(),
            "unexpected newer bytes"
        );
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(coordinator.quarantine_count().unwrap(), 1);
    }

    #[test]
    fn journal_rejects_traversal_and_oversized_targets_before_writing() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        for key in ["../escape.md", "/absolute.md", "C:/drive.md", "a\\b.md"] {
            assert!(coordinator
                .commit_local(
                    CausalStream::LocalOnly,
                    SourceChannel::parse("note_editor").unwrap(),
                    vec![TargetMutation::put(TargetKind::Markdown, key, "x")],
                    vec![draft("bad")],
                )
                .is_err());
        }
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "large.md",
                    "x".repeat(1024 * 1024 + 1),
                )],
                vec![draft("large")],
            )
            .is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert!(!temp.path().join("escape.md").exists());
    }

    #[test]
    fn overlay_target_accepts_only_a_note_id_json_leaf() {
        assert!(validate_target_key(TargetKind::OverlayJson, "note-1.json").is_ok());
        for key in [
            "ready-v1.json/escape.json",
            "overlay/notes/note.json",
            "../ready-v1.json",
            "note.md",
            ".json",
        ] {
            assert!(
                validate_target_key(TargetKind::OverlayJson, key).is_err(),
                "accepted overlay alias {key}"
            );
        }
    }

    #[test]
    fn journal_rejects_physical_path_aliases_before_staging() {
        for key in [
            "a//b.md",
            "a/./b.md",
            "a/../b.md",
            "a/b.md.",
            "a/b.md ",
            "CON.md",
            "folder/prn.txt",
            "folder/LPT9.json",
        ] {
            assert!(validate_relative_key(key).is_err(), "accepted alias {key}");
        }

        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let result = coordinator.commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("note_editor").unwrap(),
            vec![
                TargetMutation::put(TargetKind::Markdown, "Case.md", "put"),
                TargetMutation::tombstone(TargetKind::Markdown, "case.md"),
            ],
            vec![draft("case-only-move")],
        );
        assert!(result.is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert!(!vault.join("Case.md").exists());
    }

    #[test]
    fn invalid_causal_parent_fails_before_target_or_journal_and_does_not_wedge_lane() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            temp.path(),
            &vault,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let mut invalid = draft("invalid-parent");
        invalid
            .causal_parents
            .push(crate::models::twin_event::EventId::parse("f".repeat(64)).unwrap());
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "invalid.md",
                    "must not persist",
                )],
                vec![invalid],
            )
            .is_err());
        assert!(!vault.join("invalid.md").exists());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert!(store.ordered_events().unwrap().is_empty());

        let valid_commit = coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "valid.md",
                    "valid",
                )],
                vec![draft("valid-after-rejection")],
            )
            .unwrap();
        assert_eq!(valid_commit.events.len(), 1);
        assert_eq!(store.ordered_events().unwrap().len(), 1);
    }

    #[test]
    fn target_only_local_and_nonlocal_mutations_are_journaled_without_lifecycle_or_events() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let lifecycle = Arc::new(CountingLifecycle::default());
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store.clone(), lifecycle.clone())
                .unwrap();

        coordinator.fail_once_at(MutationFaultPoint::AfterTarget(0));
        let local = coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("canvas").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "layout.md",
                    "one",
                )],
                Vec::new(),
            )
            .expect("target-only local replay must converge in-call");
        assert!(local.postcommit_warning);
        assert!(lifecycle.calls.lock().unwrap().is_empty());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);

        coordinator.fail_once_at(MutationFaultPoint::AfterTarget(0));
        let remote_commit = coordinator
            .apply_nonlocal(
                crate::services::twin_events::MutationOrigin::Remote,
                vec![
                    TargetMutation::put(TargetKind::Markdown, "remote-a.md", "a"),
                    TargetMutation::put(TargetKind::Markdown, "remote-b.md", "b"),
                ],
            )
            .expect("target-only remote replay must converge in-call");
        assert!(remote_commit.events.is_empty());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(vault.join("remote-a.md")).unwrap(),
            "a"
        );
        assert_eq!(
            std::fs::read_to_string(vault.join("remote-b.md")).unwrap(),
            "b"
        );
        assert!(lifecycle.calls.lock().unwrap().is_empty());
        assert!(store.ordered_events().unwrap().is_empty());
    }

    #[test]
    fn durable_root_lease_rejects_stale_writer_before_stage_or_bytes() {
        let temp = tempdir().unwrap();
        let vault_a = temp.path().join("vault-a");
        let vault_b = temp.path().join("vault-b");
        std::fs::create_dir(&vault_a).unwrap();
        std::fs::create_dir(&vault_b).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let current = MutationCoordinator::new(
            temp.path(),
            &vault_a,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let stale = MutationCoordinator::new(
            temp.path(),
            &vault_a,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();

        current.retarget_markdown_root(&vault_b).unwrap();
        current.retarget_markdown_root(&vault_a).unwrap();
        assert!(stale
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("mcp").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "stale.md",
                    "stale"
                )],
                vec![draft("stale-writer")],
            )
            .is_err());
        assert_eq!(stale.pending_count().unwrap(), 0);
        assert!(!vault_a.join("stale.md").exists());
        assert!(!vault_b.join("stale.md").exists());
        assert!(store.ordered_events().unwrap().is_empty());
    }

    #[test]
    fn orphan_stage_temp_is_cleaned_and_does_not_wedge_recovery() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let pending = temp.path().join("twin/mutations/pending/v1");
        let orphan = pending.join(".crashed-stage.tmp");
        std::fs::write(&orphan, "partial").unwrap();

        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert!(!orphan.exists());
    }

    #[test]
    fn every_replay_fault_boundary_converges_to_one_complete_mutation() {
        let points = [
            MutationFaultPoint::AfterStage,
            MutationFaultPoint::AfterTarget(0),
            MutationFaultPoint::AfterTarget(1),
            MutationFaultPoint::AfterTargets,
            MutationFaultPoint::AfterEvent(0),
            MutationFaultPoint::AfterEvent(1),
            MutationFaultPoint::BeforeCleanup,
            MutationFaultPoint::AfterCleanupBeforeFanout,
        ];
        for point in points {
            let temp = tempdir().unwrap();
            let vault = temp.path().join("vault");
            std::fs::create_dir(&vault).unwrap();
            std::fs::write(vault.join("old.md"), "old").unwrap();
            let store = Arc::new(TwinEventStore::new(temp.path()));
            store.initialize().unwrap();
            let coordinator = MutationCoordinator::new(
                temp.path(),
                &vault,
                store.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap();
            coordinator.fail_once_at(point);
            let commit = coordinator
                .commit_local(
                    CausalStream::SyncEligible,
                    SourceChannel::parse("note_editor").unwrap(),
                    vec![
                        TargetMutation::put(TargetKind::Markdown, "new.md", "new"),
                        TargetMutation::tombstone(TargetKind::Markdown, "old.md"),
                    ],
                    vec![draft("one"), draft("two")],
                )
                .unwrap_or_else(|error| panic!("{point:?}: {error}"));
            assert!(commit.postcommit_warning, "{point:?}");
            assert_eq!(coordinator.pending_count().unwrap(), 0, "{point:?}");

            drop(coordinator);
            drop(store);
            let reopened_store = Arc::new(TwinEventStore::new(temp.path()));
            reopened_store.initialize().unwrap();
            let reopened = MutationCoordinator::new(
                temp.path(),
                &vault,
                reopened_store.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap();
            assert_eq!(reopened.recover_pending().unwrap(), 0, "{point:?}");
            drop(reopened);
            drop(reopened_store);
            let twice_reopened_store = Arc::new(TwinEventStore::new(temp.path()));
            twice_reopened_store.initialize().unwrap();
            let twice_reopened = MutationCoordinator::new(
                temp.path(),
                &vault,
                twice_reopened_store.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap();
            assert_eq!(twice_reopened.recover_pending().unwrap(), 0, "{point:?}");
            assert_eq!(
                std::fs::read_to_string(vault.join("new.md")).unwrap(),
                "new"
            );
            assert!(!vault.join("old.md").exists());
            assert_eq!(
                twice_reopened_store.ordered_events().unwrap().len(),
                2,
                "{point:?}"
            );
        }
    }

    #[derive(Default)]
    struct CountingLifecycle {
        calls: Mutex<Vec<&'static str>>,
    }

    impl crate::services::twin_events::MutationLifecycle for CountingLifecycle {
        fn stage_before_local(
            &self,
            _: &MutationIntentV1,
        ) -> Result<(), crate::services::twin_events::MutationError> {
            self.calls.lock().unwrap().push("stage");
            Ok(())
        }

        fn committed(
            &self,
            _: &MutationIntentV1,
        ) -> Result<(), crate::services::twin_events::MutationError> {
            self.calls.lock().unwrap().push("committed");
            Ok(())
        }

        fn known_failure(&self, _: Option<&str>, _: &str) {
            self.calls.lock().unwrap().push("known_failure");
        }
    }

    #[test]
    fn lifecycle_is_local_only_and_remote_recovery_never_echo() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let lifecycle = Arc::new(CountingLifecycle::default());
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store.clone(), lifecycle.clone())
                .unwrap();

        let remote_commit = coordinator
            .apply_nonlocal(
                crate::services::twin_events::MutationOrigin::Remote,
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "remote.md",
                    "remote",
                )],
            )
            .unwrap();
        assert!(remote_commit.events.is_empty());
        assert!(lifecycle.calls.lock().unwrap().is_empty());
        assert!(store.ordered_events().unwrap().is_empty());

        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        let local = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "local.md",
                    "local",
                )],
                vec![draft("local")],
            )
            .expect("the exact staged mutation must converge in-call");
        assert!(local.postcommit_warning);
        assert_eq!(*lifecycle.calls.lock().unwrap(), vec!["stage", "committed"]);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert_eq!(*lifecycle.calls.lock().unwrap(), vec!["stage", "committed"]);
        assert_eq!(store.ordered_events().unwrap().len(), 1);

        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "too-large.md",
                    "x".repeat(MAX_MARKDOWN_TWIN_BYTES + 1),
                )],
                vec![draft("too-large")],
            )
            .is_err());
        assert_eq!(
            *lifecycle.calls.lock().unwrap(),
            vec!["stage", "committed", "known_failure"]
        );
    }

    #[test]
    fn mutation_identity_binds_outer_actor_and_source_but_not_audit_time() {
        let intent = legacy_markdown_intent(
            digest_bytes(b"root"),
            "note.md",
            BeforeImage::Absent,
            "note",
        );

        let mut altered_source = intent.clone();
        altered_source.source_channel = SourceChannel::parse("mcp").unwrap();
        assert_ne!(derive_mutation_id(&altered_source), intent.mutation_id);
        altered_source.mutation_id = derive_mutation_id(&altered_source);
        altered_source.validate().unwrap();

        let mut altered_actor = intent.clone();
        altered_actor.actor_id = crate::models::twin_event::ActorId::parse("other-owner").unwrap();
        assert_ne!(derive_mutation_id(&altered_actor), intent.mutation_id);

        let mut altered_scope = intent.clone();
        altered_scope.markdown_root_scope = Some(digest_bytes(b"other-root"));
        assert_ne!(derive_mutation_id(&altered_scope), intent.mutation_id);

        let mut missing_scope = serde_json::to_value(&intent).unwrap();
        missing_scope
            .as_object_mut()
            .unwrap()
            .remove("markdown_root_scope");
        assert!(serde_json::from_value::<MutationIntentV1>(missing_scope).is_err());

        let mut altered_audit_time = intent.clone();
        altered_audit_time.created_at += chrono::Duration::days(1);
        assert_eq!(derive_mutation_id(&altered_audit_time), intent.mutation_id);
    }

    #[test]
    fn retarget_recovers_pending_markdown_under_the_old_root_before_switching() {
        let temp = tempdir().unwrap();
        let vault_a = temp.path().join("vault-a");
        let vault_b = temp.path().join("vault-b");
        std::fs::create_dir(&vault_a).unwrap();
        std::fs::create_dir(&vault_b).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            temp.path(),
            &vault_a,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();

        let scope = coordinator.current_authority_token().unwrap().root_scope;
        let pending = legacy_markdown_intent(
            scope,
            "interrupted.md",
            BeforeImage::Absent,
            "old-root bytes",
        );
        stage_manual_intent(temp.path(), &coordinator, &pending);
        assert_eq!(coordinator.pending_count().unwrap(), 1);

        coordinator.retarget_markdown_root(&vault_b).unwrap();
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(vault_a.join("interrupted.md")).unwrap(),
            "old-root bytes"
        );
        assert!(!vault_b.join("interrupted.md").exists());

        let switched_commit = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "after-switch.md",
                    "new-root bytes",
                )],
                vec![draft("after-switch")],
            )
            .unwrap();
        assert_eq!(switched_commit.events.len(), 1);
        assert!(!vault_a.join("after-switch.md").exists());
        assert_eq!(
            std::fs::read_to_string(vault_b.join("after-switch.md")).unwrap(),
            "new-root bytes"
        );
        assert_eq!(store.ordered_events().unwrap().len(), 1);
    }

    #[test]
    fn stale_markdown_root_intent_is_quarantined_without_touching_new_root() {
        let temp = tempdir().unwrap();
        let vault_a = temp.path().join("vault-a");
        let vault_b = temp.path().join("vault-b");
        std::fs::create_dir(&vault_a).unwrap();
        std::fs::create_dir(&vault_b).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let current = MutationCoordinator::new(
            temp.path(),
            &vault_a,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let stale = MutationCoordinator::new(
            temp.path(),
            &vault_a,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        current.retarget_markdown_root(&vault_b).unwrap();

        assert!(stale
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("mcp").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "stale.md",
                    "stale-root bytes",
                )],
                vec![draft("stale")],
            )
            .is_err());

        assert_eq!(current.recover_pending().unwrap(), 0);
        assert!(!vault_a.join("stale.md").exists());
        assert!(!vault_b.join("stale.md").exists());
        assert_eq!(current.pending_count().unwrap(), 0);
        assert_eq!(current.quarantine_count().unwrap(), 0);
        assert!(store.ordered_events().unwrap().is_empty());
    }

    #[test]
    fn desktop_and_mcp_coordinators_serialize_one_device_lane_without_duplicates() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let desktop = Arc::new(
            MutationCoordinator::new(
                temp.path(),
                &vault,
                store.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mcp = Arc::new(
            MutationCoordinator::new(
                temp.path(),
                &vault,
                store.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let desktop_thread = {
            let coordinator = desktop.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                coordinator.commit_local(
                    CausalStream::SyncEligible,
                    SourceChannel::parse("note_editor").unwrap(),
                    vec![TargetMutation::put(
                        TargetKind::Markdown,
                        "desktop.md",
                        "desktop",
                    )],
                    vec![draft("desktop")],
                )
            })
        };
        let mcp_thread = {
            let coordinator = mcp.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                coordinator.commit_local(
                    CausalStream::SyncEligible,
                    SourceChannel::parse("mcp").unwrap(),
                    vec![TargetMutation::put(TargetKind::Markdown, "mcp.md", "mcp")],
                    vec![draft("mcp")],
                )
            })
        };
        barrier.wait();
        let desktop_commit = desktop_thread.join().unwrap().unwrap();
        let mcp_commit = mcp_thread.join().unwrap().unwrap();
        assert_eq!(desktop_commit.events.len(), 1);
        assert_eq!(mcp_commit.events.len(), 1);

        assert_eq!(
            std::fs::read_to_string(vault.join("desktop.md")).unwrap(),
            "desktop"
        );
        assert_eq!(
            std::fs::read_to_string(vault.join("mcp.md")).unwrap(),
            "mcp"
        );
        let events = store
            .ordered_events_for_stream(CausalStream::SyncEligible)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].device_id, events[1].device_id);
        assert_eq!(events[0].device_sequence, 1);
        assert_eq!(events[1].device_sequence, 2);
        assert!(events[1].causal_parents.contains(&events[0].event_id));
        assert_ne!(events[0].event_id, events[1].event_id);
    }

    #[test]
    fn corrupt_pending_and_pending_cap_fail_closed() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let pending = temp.path().join("twin/mutations/pending/v1");
        std::fs::write(pending.join("corrupt.json"), "{not-json").unwrap();
        assert!(coordinator.recover_pending().is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(coordinator.quarantine_count().unwrap(), 1);

        for index in 0..=MAX_PENDING_INTENTS {
            std::fs::write(pending.join(format!("{index:064x}.json")), "{}").unwrap();
        }
        assert!(coordinator.pending_count().is_err());
    }

    #[test]
    fn journal_directory_caps_count_temporary_and_quarantine_entries() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let journal = LocalMutationJournal::initialize(temp.path()).unwrap();

        let staging = temp.path().join(STAGING_DIRECTORY.replace('/', "\\"));
        for index in 0..=MAX_PENDING_INTENTS {
            std::fs::write(staging.join(format!(".{index}.tmp")), b"temp").unwrap();
        }
        assert!(journal
            .cleanup_orphan_temps_locked(guard.process_lock())
            .is_err());
        assert_eq!(
            std::fs::read_dir(&staging).unwrap().count(),
            MAX_PENDING_INTENTS + 1
        );

        let quarantine = temp.path().join(QUARANTINE_DIRECTORY.replace('/', "\\"));
        for index in 0..=MAX_PENDING_INTENTS {
            std::fs::write(quarantine.join(format!("{index:064x}.json")), b"{}").unwrap();
        }
        assert!(journal.quarantine_count(guard.process_lock()).is_err());
    }

    #[test]
    fn symlinked_target_component_is_rejected_without_escape() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let link = vault.join("jump");
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(&outside, &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("failed to create test symlink: {error}");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "jump/escape.md",
                    "x"
                )],
                vec![draft("symlink")],
            )
            .is_err());
        assert!(!outside.join("escape.md").exists());
    }
}

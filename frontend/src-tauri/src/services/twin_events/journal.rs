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
        }
    }

    pub fn tombstone(kind: TargetKind, relative_key: impl Into<String>) -> Self {
        Self {
            kind,
            relative_key: relative_key.into(),
            after: DesiredImage::Tombstone,
            expected_before: None,
        }
    }

    pub fn expecting(mut self, expected_before: BeforeImage) -> Self {
        self.expected_before = Some(expected_before);
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
    pub targets: Vec<MutationTargetV1>,
    pub events: Vec<TwinEvent>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl MutationIntentV1 {
    pub fn validate(&self) -> Result<(), MutationError> {
        if !matches!(self.schema_version, 1 | 2) {
            return Err(MutationError::Invalid(
                "unsupported local mutation journal schema".into(),
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
        let has_vault_scoped_target = self
            .targets
            .iter()
            .any(|target| matches!(target.kind, TargetKind::Markdown | TargetKind::OverlayJson));
        if self.markdown_root_scope.is_some() != has_vault_scoped_target {
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
            2 if changes_authority
                && self
                    .content_authority_generation
                    .is_some_and(|generation| generation != u64::MAX) => {}
            2 if !changes_authority && self.content_authority_generation.is_none() => {}
            2 => {
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
const QUARANTINE_DIRECTORY: &str = "twin/mutations/quarantine/v1";
const STAGING_DIRECTORY: &str = "twin/mutations/staging/v1";

pub struct LocalMutationJournal {
    root: crate::services::twin_events::AnchoredRoot,
}

impl LocalMutationJournal {
    pub fn initialize(data_path: impl AsRef<Path>) -> Result<Self, MutationError> {
        let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
        root.open_directory(PENDING_DIRECTORY, true)?;
        root.open_directory(QUARANTINE_DIRECTORY, true)?;
        root.open_directory(STAGING_DIRECTORY, true)?;
        Ok(Self { root })
    }

    pub(crate) fn pending_count(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<usize, MutationError> {
        Ok(self.pending_paths()?.len())
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
        let mut bytes = serde_json::to_vec_pretty(intent)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_SERIALIZED_INTENT_BYTES {
            return Err(MutationError::Invalid(
                "serialized mutation intent exceeds the 32 MiB limit".into(),
            ));
        }
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

    fn quarantine_path(&self, source: &str) -> Result<(), MutationError> {
        let name = source.rsplit('/').next().unwrap_or("intent.json");
        let target = format!("{QUARANTINE_DIRECTORY}/{}-{name}", Uuid::new_v4());
        self.root.rename(source, &target, false)
    }

    pub(crate) fn cleanup_orphan_temps_locked(
        &self,
        _lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<(), MutationError> {
        for directory in [PENDING_DIRECTORY, STAGING_DIRECTORY] {
            for name in self.root.regular_file_names(directory)? {
                if name.starts_with('.') && name.ends_with(".tmp") {
                    self.root.delete(&format!("{directory}/{name}"))?;
                }
            }
        }
        Ok(())
    }

    fn json_names(&self, directory: &str) -> Result<Vec<String>, MutationError> {
        let names = self.root.regular_file_names(directory)?;
        if let Some(name) = names.iter().find(|name| !name.ends_with(".json")) {
            return Err(MutationError::Invalid(format!(
                "mutation journal contains a non-JSON entry: {name}"
            )));
        }
        Ok(names)
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

    #[test]
    fn schema_two_authority_generation_is_required_and_changes_mutation_identity() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            temp.path(),
            &vault,
            store,
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "generation.md",
                    "after",
                )],
                vec![draft("generation")],
            )
            .is_err());

        let pending = temp.path().join(PENDING_DIRECTORY);
        let path = std::fs::read_dir(&pending)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let bytes = std::fs::read(path).unwrap();
        let intent: MutationIntentV1 = serde_json::from_slice(&bytes).unwrap();
        intent.validate().unwrap();
        let original_id = intent.mutation_id.clone();

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
        let result = coordinator.commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![
                TargetMutation::put(TargetKind::Markdown, "new.md", "new"),
                TargetMutation::tombstone(TargetKind::Markdown, "old.md"),
            ],
            vec![draft("move")],
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(vault.join("new.md")).unwrap(),
            "new"
        );
        assert!(vault.join("old.md").exists());
        assert!(store.ordered_events().unwrap().is_empty());

        assert_eq!(coordinator.recover_pending().unwrap(), 1);
        assert!(!vault.join("old.md").exists());
        assert_eq!(store.ordered_events().unwrap().len(), 1);
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
        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "note.md",
                    "desired",
                )],
                vec![draft("note")],
            )
            .is_err());
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

        coordinator
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
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("canvas").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "layout.md",
                    "one"
                )],
                Vec::new(),
            )
            .is_err());
        assert!(lifecycle.calls.lock().unwrap().is_empty());
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 1);

        coordinator.fail_once_at(MutationFaultPoint::AfterTarget(0));
        assert!(coordinator
            .apply_nonlocal(
                crate::services::twin_events::MutationOrigin::Remote,
                vec![
                    TargetMutation::put(TargetKind::Markdown, "remote-a.md", "a"),
                    TargetMutation::put(TargetKind::Markdown, "remote-b.md", "b"),
                ],
            )
            .is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 1);
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
    fn every_crash_boundary_converges_to_one_complete_mutation() {
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
            assert!(
                coordinator
                    .commit_local(
                        CausalStream::SyncEligible,
                        SourceChannel::parse("note_editor").unwrap(),
                        vec![
                            TargetMutation::put(TargetKind::Markdown, "new.md", "new"),
                            TargetMutation::tombstone(TargetKind::Markdown, "old.md"),
                        ],
                        vec![draft("one"), draft("two")],
                    )
                    .is_err(),
                "{point:?}"
            );

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
            let recovered = reopened.recover_pending().unwrap();
            if point == MutationFaultPoint::AfterCleanupBeforeFanout {
                assert_eq!(recovered, 0, "{point:?}");
            } else {
                assert_eq!(recovered, 1, "{point:?}");
            }
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

        coordinator
            .apply_nonlocal(
                crate::services::twin_events::MutationOrigin::Remote,
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "remote.md",
                    "remote",
                )],
            )
            .unwrap();
        assert!(lifecycle.calls.lock().unwrap().is_empty());
        assert!(store.ordered_events().unwrap().is_empty());

        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "local.md",
                    "local"
                )],
                vec![draft("local")],
            )
            .is_err());
        assert_eq!(
            *lifecycle.calls.lock().unwrap(),
            vec!["stage", "known_failure"]
        );
        coordinator.recover_pending().unwrap();
        assert_eq!(
            *lifecycle.calls.lock().unwrap(),
            vec!["stage", "known_failure"]
        );
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
            vec!["stage", "known_failure", "known_failure"]
        );
    }

    #[test]
    fn mutation_identity_binds_outer_actor_and_source_but_not_audit_time() {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(temp.path()));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();
        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(TargetKind::Markdown, "note.md", "note")],
                vec![draft("note")],
            )
            .is_err());
        let pending = std::fs::read_dir(temp.path().join("twin/mutations/pending/v1"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let intent: MutationIntentV1 =
            serde_json::from_slice(&std::fs::read(pending).unwrap()).unwrap();

        let mut altered_source = intent.clone();
        altered_source.source_channel = SourceChannel::parse("mcp").unwrap();
        assert_ne!(derive_mutation_id(&altered_source), intent.mutation_id);
        altered_source.mutation_id = derive_mutation_id(&altered_source);
        assert!(altered_source.validate().is_err());

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

        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    "interrupted.md",
                    "old-root bytes",
                )],
                vec![draft("interrupted")],
            )
            .is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 1);

        coordinator.retarget_markdown_root(&vault_b).unwrap();
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(vault_a.join("interrupted.md")).unwrap(),
            "old-root bytes"
        );
        assert!(!vault_b.join("interrupted.md").exists());

        coordinator
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
        assert!(!vault_a.join("after-switch.md").exists());
        assert_eq!(
            std::fs::read_to_string(vault_b.join("after-switch.md")).unwrap(),
            "new-root bytes"
        );
        assert_eq!(store.ordered_events().unwrap().len(), 2);
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
        desktop_thread.join().unwrap().unwrap();
        mcp_thread.join().unwrap().unwrap();

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

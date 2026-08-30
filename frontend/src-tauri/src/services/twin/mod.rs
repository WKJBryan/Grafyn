//! Twin evidence store: sealed decisions, Constitution, action gaps, memory
//! digest, session traces, and export. Split into focused submodules (Task 4.2):
//! - `shared` -- quarantine/load-or-quarantine file helpers and cross-cutting
//!   text/payload utilities used by every domain submodule below.
//! - `traces` -- session trace append/read and evidence-ref resolution.
//! - `records` -- user record CRUD, promotion, and behavioral inference.
//! - `constitution` -- Constitution items, action gaps, guided setup, and
//!   constitution inference (including interview-note extraction).
//! - `decisions` -- decision episodes, reflection cards, and sealed twin
//!   predictions (seal-integrity logic intact, see `attach_twin_prediction`).
//! - `digest` -- memory digest clustering and review.
//! - `export` -- the JSONL/manifest export bundle.
//!
//! `TwinStore`'s fields and the few helpers used by every domain (`new`,
//! `write_pretty_json`, `validate_file_id`) stay here so they're visible to
//! all of the above (private items defined in a parent module are visible to
//! descendant modules in Rust -- no `pub(super)` needed for these). Public
//! method signatures on `TwinStore` are unchanged from the pre-split
//! `twin_store.rs` -- callers in `commands/twin.rs` and
//! `commands/canvas/context.rs` do not need to change.

mod constitution;
mod decisions;
mod feedback;
mod records;
mod shared;
mod traces;
// The mcp binary (grafyn-mcp) compiles this whole module tree but has no
// caller for this re-export -- only the desktop app's commands/canvas/context.rs
// imports `crate::services::twin::parse_twin_prediction`.
#[allow(unused_imports)]
pub use decisions::parse_twin_prediction;
mod digest;
mod export;

use crate::models::twin::{SessionTrace, UserRecord};
use crate::services::atomic_io::write_atomic;
use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const AUTO_PROMOTE_CONFIDENCE: f32 = 0.75;
const AUTO_PROMOTE_SUPPORT_COUNT: usize = 3;

pub struct TwinStore {
    root_path: PathBuf,
    target_root_path: PathBuf,
    traces_path: PathBuf,
    records_path: PathBuf,
    decisions_path: PathBuf,
    reflections_path: PathBuf,
    constitution_path: PathBuf,
    action_gaps_path: PathBuf,
    setup_path: PathBuf,
    decision_mirror_config_path: PathBuf,
    digest_path: PathBuf,
    exports_path: PathBuf,
    trace_cache: HashMap<String, SessionTrace>,
    record_cache: HashMap<String, UserRecord>,
    records_cache_ready: bool,
    event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
    root_capability: Option<crate::services::twin_events::AnchoredRoot>,
    data_capability: Option<crate::services::twin_events::AnchoredRoot>,
}

impl TwinStore {
    pub fn new(root_path: PathBuf) -> Self {
        Self::with_event_recorder(
            root_path.clone(),
            root_path,
            Arc::new(crate::services::twin_events::NoopEventRecorder),
        )
    }

    pub fn with_event_recorder(
        root_path: PathBuf,
        target_root_path: PathBuf,
        event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
    ) -> Self {
        let traces_path = root_path.join("traces");
        let records_path = root_path.join("records");
        let decisions_path = root_path.join("decisions");
        let reflections_path = root_path.join("reflections");
        let constitution_path = root_path.join("constitution");
        let action_gaps_path = root_path.join("action_gaps");
        let setup_path = root_path.join("constitution_setup.json");
        let decision_mirror_config_path = root_path.join("decision_mirror_config.json");
        let digest_path = root_path.join("memory_digest.json");
        let exports_path = root_path.join("exports");

        std::fs::create_dir_all(&traces_path).ok();
        std::fs::create_dir_all(&records_path).ok();
        std::fs::create_dir_all(&decisions_path).ok();
        std::fs::create_dir_all(&reflections_path).ok();
        std::fs::create_dir_all(&constitution_path).ok();
        std::fs::create_dir_all(&action_gaps_path).ok();
        std::fs::create_dir_all(&exports_path).ok();
        let root_capability = crate::services::twin_events::AnchoredRoot::open(&root_path).ok();
        let data_capability = target_root_path
            .parent()
            .and_then(|path| crate::services::twin_events::AnchoredRoot::open(path).ok());

        Self {
            root_path,
            target_root_path,
            traces_path,
            records_path,
            decisions_path,
            reflections_path,
            constitution_path,
            action_gaps_path,
            setup_path,
            decision_mirror_config_path,
            digest_path,
            exports_path,
            trace_cache: HashMap::new(),
            record_cache: HashMap::new(),
            records_cache_ready: false,
            event_recorder,
            root_capability,
            data_capability,
        }
    }

    pub fn replace_root_path(&mut self, root_path: PathBuf) -> Result<()> {
        std::fs::create_dir_all(&root_path)
            .with_context(|| format!("Failed to create Twin root: {}", root_path.display()))?;
        crate::services::twin_events::validate_real_directory(&root_path, "Twin root")
            .map_err(anyhow::Error::new)?;
        *self = Self::with_event_recorder(
            root_path,
            self.target_root_path.clone(),
            self.event_recorder.clone(),
        );
        Ok(())
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn target_root_path(&self) -> &Path {
        &self.target_root_path
    }

    fn write_pretty_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let content = serde_json::to_string_pretty(value)?;
        if self.event_recorder.is_noop() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
            return write_atomic(path, content.as_bytes())
                .with_context(|| format!("Failed to write JSON file: {}", path.display()));
        }
        self.commit_governed_json_targets(vec![(path.to_path_buf(), content)], Vec::new())
    }

    fn governed_json_digest<T: Serialize>(
        value: &T,
    ) -> Result<crate::models::twin_event::ContentDigest> {
        Ok(crate::services::twin_events::digest_bytes(
            serde_json::to_string_pretty(value)?.as_bytes(),
        ))
    }

    fn read_twin_json_bounded<T: DeserializeOwned>(&self, path: &Path) -> Result<Option<T>> {
        const TWIN_JSON_LIMIT: usize = 1024 * 1024;
        let relative = path
            .strip_prefix(&self.root_path)
            .map_err(|_| anyhow::anyhow!("Twin read escaped the configured store root"))?
            .to_string_lossy()
            .replace('\\', "/");
        let root = self.root_capability.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Twin store root capability could not be acquired")
        })?;
        let Some(bytes) = root
            .read_bounded(&relative, TWIN_JSON_LIMIT)
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse Twin JSON file: {}", path.display()))
            .map(Some)
    }

    fn list_twin_json_bounded<T: DeserializeOwned>(
        &self,
        relative_directory: &str,
    ) -> Result<Vec<T>> {
        let root = self.root_capability.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Twin store root capability could not be acquired")
        })?;
        let names = root
            .regular_file_names(relative_directory)
            .map_err(anyhow::Error::new)?;
        let mut values = Vec::new();
        for name in names
            .into_iter()
            .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "json"))
        {
            let relative = format!("{relative_directory}/{name}");
            let bytes = match root.read_bounded(&relative, 1024 * 1024) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(error) => {
                    log::error!("Skipping unreadable Twin JSON file {relative}: {error}");
                    continue;
                }
            };
            let value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(error) => {
                    log::error!(
                        "Skipping corrupt Twin JSON file {relative}: {error} — quarantining"
                    );
                    let quarantine = format!("{relative}.corrupt-{}", uuid::Uuid::new_v4());
                    if let Err(rename_error) = root.rename(&relative, &quarantine, false) {
                        log::error!(
                            "Failed to quarantine corrupt Twin JSON file {relative}: {rename_error}"
                        );
                    }
                    continue;
                }
            };
            values.push(value);
        }
        Ok(values)
    }

    fn read_canvas_session_bounded(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::models::canvas::CanvasSession>> {
        const CANVAS_JSON_LIMIT: usize = 16 * 1024 * 1024;
        Self::validate_file_id(session_id)?;
        let root = self.data_capability.as_ref().ok_or_else(|| {
            anyhow::anyhow!("coordinator data-root capability could not be acquired")
        })?;
        let key = format!("canvas/{session_id}.json");
        let Some(bytes) = root
            .read_bounded(&key, CANVAS_JSON_LIMIT)
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .context("Failed to parse persisted Canvas session")
            .map(Some)
    }

    fn invalidate_mutation_caches(&mut self) {
        self.trace_cache.clear();
        self.record_cache.clear();
        self.records_cache_ready = false;
    }

    fn commit_planned_twin_mutation<F>(
        &mut self,
        mut planner: F,
    ) -> Result<crate::services::twin_events::MutationCommit>
    where
        F: FnMut(&Self) -> Result<Option<crate::services::twin_events::MutationPlan>>,
    {
        let recorder = self.event_recorder.clone();
        let result = {
            let mut adapter = || {
                planner(self).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
            };
            recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut adapter,
            )
        };
        match result {
            Ok(commit) => Ok(commit),
            Err(error) => {
                self.invalidate_mutation_caches();
                Err(anyhow::Error::new(error))
            }
        }
    }

    fn write_governed_json<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
        drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<()> {
        let content = serde_json::to_string_pretty(value)?;
        self.commit_governed_json_targets(vec![(path.to_path_buf(), content)], drafts)
    }

    fn delete_governed_json(
        &self,
        path: &Path,
        drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<()> {
        if self.event_recorder.is_noop() {
            match std::fs::remove_file(path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        let relative = path
            .strip_prefix(&self.target_root_path)
            .map_err(|_| anyhow::anyhow!("Twin mutation target escaped the configured Twin root"))?
            .to_string_lossy()
            .replace('\\', "/");
        self.event_recorder
            .commit_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                crate::models::twin_event::CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(anyhow::Error::msg)?,
                vec![crate::services::twin_events::TargetMutation::tombstone(
                    crate::services::twin_events::TargetKind::TwinJson,
                    relative,
                )],
                drafts,
            )
            .map_err(anyhow::Error::new)?;
        Ok(())
    }

    fn commit_governed_json_targets(
        &self,
        values: Vec<(PathBuf, String)>,
        drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<()> {
        self.commit_governed_json_targets_with_source(
            crate::models::twin_event::SourceChannel::parse("legacy_twin")
                .map_err(anyhow::Error::msg)?,
            values,
            drafts,
        )
    }

    fn commit_governed_json_targets_with_source(
        &self,
        source_channel: crate::models::twin_event::SourceChannel,
        values: Vec<(PathBuf, String)>,
        drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<()> {
        if self.event_recorder.is_noop() {
            for (path, content) in values {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                write_atomic(&path, content.as_bytes())?;
            }
            return Ok(());
        }
        let targets = self.governed_json_targets(values)?;
        self.event_recorder
            .commit_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                crate::models::twin_event::CausalStream::SyncEligible,
                source_channel,
                targets,
                drafts,
            )
            .map_err(anyhow::Error::new)?;
        Ok(())
    }

    pub(crate) fn governed_json_targets(
        &self,
        values: Vec<(PathBuf, String)>,
    ) -> Result<Vec<crate::services::twin_events::TargetMutation>> {
        values
            .into_iter()
            .map(|(path, content)| {
                let relative = path
                    .strip_prefix(&self.target_root_path)
                    .map_err(|_| {
                        anyhow::anyhow!("Twin mutation target escaped the configured Twin root")
                    })?
                    .to_string_lossy()
                    .replace('\\', "/");
                Ok(crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::TwinJson,
                    relative,
                    content,
                ))
            })
            .collect::<Result<Vec<_>>>()
    }

    fn governed_tombstone_target(
        &self,
        path: &Path,
    ) -> Result<crate::services::twin_events::TargetMutation> {
        let relative = path
            .strip_prefix(&self.target_root_path)
            .map_err(|_| anyhow::anyhow!("Twin mutation target escaped the configured Twin root"))?
            .to_string_lossy()
            .replace('\\', "/");
        Ok(crate::services::twin_events::TargetMutation::tombstone(
            crate::services::twin_events::TargetKind::TwinJson,
            relative,
        ))
    }

    fn validate_file_id(id: &str) -> Result<()> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid file id: {}", id);
        }

        Ok(())
    }
}

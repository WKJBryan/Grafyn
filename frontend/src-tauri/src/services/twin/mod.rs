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
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const AUTO_PROMOTE_CONFIDENCE: f32 = 0.75;
const AUTO_PROMOTE_SUPPORT_COUNT: usize = 3;

pub struct TwinStore {
    root_path: PathBuf,
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
}

impl TwinStore {
    pub fn new(root_path: PathBuf) -> Self {
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

        Self {
            root_path,
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
        }
    }

    fn write_pretty_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(value)?;
        write_atomic(path, content.as_bytes())
            .with_context(|| format!("Failed to write JSON file: {}", path.display()))
    }

    fn validate_file_id(id: &str) -> Result<()> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid file id: {}", id);
        }

        Ok(())
    }
}

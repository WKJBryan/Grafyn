use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownMigrationMode {
    #[default]
    SidecarFirst,
    Hybrid,
    FullRewrite,
}

impl MarkdownMigrationMode {
    pub fn allows_user_note_writes(&self) -> bool {
        matches!(self, Self::Hybrid | Self::FullRewrite)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaultOptimizerEditMode {
    #[default]
    SidecarFirst,
    Hybrid,
    FullRewrite,
}

#[allow(dead_code)]
impl VaultOptimizerEditMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SidecarFirst => "sidecar_first",
            Self::Hybrid => "hybrid",
            Self::FullRewrite => "full_rewrite",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationRequest {
    #[serde(default)]
    pub mode: MarkdownMigrationMode,
    #[serde(default)]
    pub hub_folder: Option<String>,
    #[serde(default)]
    pub start_optimizer: Option<bool>,
    #[serde(default)]
    pub enable_llm: Option<bool>,
    #[serde(default)]
    pub auto_insert_links: Option<bool>,
    #[serde(default)]
    pub program_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationPreviewSummary {
    pub total_scanned_notes: usize,
    pub files_without_frontmatter: usize,
    pub inferred_titles: usize,
    pub inferred_aliases: usize,
    pub inferred_tags_or_topic_seeds: usize,
    pub markdown_links_resolved: usize,
    pub wikilinks_resolved: usize,
    pub proposed_hubs: usize,
    pub proposed_auto_link_edits: usize,
    pub ambiguous_matches: usize,
    pub files_to_rewrite: usize,
    pub files_to_create: usize,
    pub old_grafyn_notes_eligible_for_backfill: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationTopicCandidate {
    pub topic_key: String,
    pub display_name: String,
    #[serde(default)]
    pub member_note_ids: Vec<String>,
    #[serde(default)]
    pub member_note_titles: Vec<String>,
    #[serde(default)]
    pub reuse_existing_hub_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationNoteProposal {
    pub note_id: String,
    pub title: String,
    pub relative_path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub inferred_tags: Vec<String>,
    #[serde(default)]
    pub inferred_link_ids: Vec<String>,
    #[serde(default)]
    pub topic_key: Option<String>,
    pub confidence: f64,
    #[serde(default)]
    pub write_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationPreview {
    pub preview_id: String,
    pub vault_path: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub mode: MarkdownMigrationMode,
    #[serde(default)]
    pub hub_folder: String,
    #[serde(default)]
    pub program_path: String,
    #[serde(default)]
    pub summary: MarkdownMigrationPreviewSummary,
    #[serde(default)]
    pub topic_candidates: Vec<MarkdownMigrationTopicCandidate>,
    #[serde(default)]
    pub note_proposals: Vec<MarkdownMigrationNoteProposal>,
    #[serde(default)]
    pub ambiguous_titles: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationApplyResult {
    pub run_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_hub_note_ids: Vec<String>,
    #[serde(default)]
    pub touched_note_ids: Vec<String>,
    #[serde(default)]
    pub overlay_note_ids: Vec<String>,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownMigrationStatus {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub preview_id: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub mode: Option<MarkdownMigrationMode>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub applied_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub rollback_available: bool,
    #[serde(default)]
    pub summary: Option<MarkdownMigrationPreviewSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultOptimizerSettingsUpdate {
    pub background_vault_optimizer_enabled: Option<bool>,
    pub background_vault_optimizer_llm_enabled: Option<bool>,
    pub background_vault_optimizer_budget_monthly: Option<u32>,
    pub background_vault_optimizer_max_daily_writes: Option<u32>,
    pub background_vault_optimizer_edit_mode: Option<String>,
    pub background_vault_optimizer_program_enabled: Option<bool>,
    pub vault_optimizer_program_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultOptimizerDecision {
    pub id: String,
    #[serde(default)]
    pub note_id: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub diff_preview: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultOptimizerInboxEntry {
    pub id: String,
    #[serde(default)]
    pub note_id: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub diff_preview: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultOptimizerStatus {
    pub enabled: bool,
    pub llm_enabled: bool,
    pub edit_mode: String,
    pub queue_size: usize,
    #[serde(default)]
    pub last_run_at: Option<DateTime<Utc>>,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub rollback_count: usize,
    pub inbox_count: usize,
    #[serde(default)]
    pub recent_auto_edits: Vec<VaultOptimizerDecision>,
    #[serde(default)]
    pub rollback_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultOptimizerRollbackResult {
    pub change_id: String,
    #[serde(default)]
    pub rolled_back: bool,
    #[serde(default)]
    pub message: String,
}

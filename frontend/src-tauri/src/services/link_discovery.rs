use crate::models::link_discovery::{
    DismissLinkSuggestionResponse, LinkDiscoveryStatus, LinkSuggestionQueueEntry,
};
use crate::models::note::{
    ChunkResult, DiscoverLinksResponse, Note, TopicHubCandidate, ZettelLinkCandidate,
};
use crate::models::settings::UserSettings;
use crate::services::retrieval::RetrievalResult;
use crate::services::similarity::{SimilarityProvider, TfIdfProvider};
use crate::services::yake::{self, YakeConfig, STOPWORDS};
use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
#[cfg(feature = "tauri-app")]
use std::time::Instant;

#[cfg(feature = "tauri-app")]
use crate::services::openrouter::ChatMessage;
#[cfg(feature = "tauri-app")]
use crate::AppState;

const MAX_LOCAL_CANDIDATES: usize = 40;
const DEFAULT_CHUNK_LIMIT: usize = 60;
const PROFILE_VECTOR_TEXT_LIMIT: usize = 1600;

lazy_static! {
    static ref PROPER_NOUN_RE: Regex = Regex::new(r"\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+\b").unwrap();
    static ref ACRONYM_RE: Regex = Regex::new(r"\b[A-Z]{2,}\b").unwrap();
    static ref WIKILINK_RE: Regex = Regex::new(r"\[\[.+?\]\]").unwrap();
    static ref FENCED_CODE_RE: Regex = Regex::new(r"(?s)```.*?```").unwrap();
    static ref INLINE_CODE_RE: Regex = Regex::new(r"`[^`]+`").unwrap();
    static ref JSON_ARRAY_RE: Regex = Regex::new(r"(?s)\[.*\]").unwrap();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverMode {
    Manual,
    Algorithm,
    Llm,
}

impl DiscoverMode {
    pub fn parse(mode: Option<&str>) -> Self {
        match mode.unwrap_or("suggested").to_ascii_lowercase().as_str() {
            "manual" => Self::Manual,
            "algorithm" => Self::Algorithm,
            "llm" | "suggested" => Self::Llm,
            _ => Self::Llm,
        }
    }

    pub fn include_llm(self) -> bool {
        matches!(self, Self::Llm)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkDiscoveryProfile {
    pub note_id: String,
    pub title: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub topic_key: Option<String>,
    #[serde(default)]
    pub topic_hub_ids: Vec<String>,
    #[serde(default)]
    pub is_topic_hub: bool,
    pub summary: String,
    pub key_terms: Vec<String>,
    pub term_vector: HashMap<String, f64>,
    pub existing_link_ids: Vec<String>,
    pub content_hash: String,
    pub updated_at: DateTime<Utc>,
    pub last_discovered_at: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredDiscoveryNote {
    profile: LinkDiscoveryProfile,
    #[serde(default)]
    links: Vec<ZettelLinkCandidate>,
    #[serde(default)]
    exploratory_links: Vec<ZettelLinkCandidate>,
    #[serde(default)]
    cached_at: Option<DateTime<Utc>>,
    #[serde(default)]
    is_stale: bool,
    #[serde(default)]
    dismissed_target_ids: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedQueueState {
    #[serde(default)]
    queue: Vec<QueuedNote>,
    #[serde(default)]
    sweep_cursor: usize,
    #[serde(default)]
    last_run_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedNote {
    note_id: String,
    priority: QueuePriority,
    enqueued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum QueuePriority {
    Dirty,
    Unlinked,
    Stale,
    Sweep,
}

impl QueuePriority {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dirty => "dirty",
            Self::Unlinked => "unlinked",
            Self::Stale => "stale",
            Self::Sweep => "sweep",
        }
    }

    fn is_high_priority(self) -> bool {
        matches!(self, Self::Dirty | Self::Unlinked)
    }
}

#[derive(Debug, Clone)]
pub struct DiscoverySnapshot {
    pub source_profile: LinkDiscoveryProfile,
    pub dismissed_target_ids: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct BackgroundDiscoveryJob {
    pub note_id: String,
    pub mode: DiscoverMode,
    pub priority: String,
}

#[derive(Debug, Clone, Default)]
pub struct LocalCandidateSignals {
    pub target_id: String,
    pub target_title: String,
    pub summary: String,
    pub snippet: String,
    pub note_score: f64,
    pub chunk_score: f64,
    pub tag_overlap: usize,
    pub tag_overlap_ratio: f64,
    pub key_term_cosine: f64,
    pub graph_proximity: f64,
}

#[derive(Debug, Clone)]
pub struct RankedLinkCandidate {
    pub candidate: ZettelLinkCandidate,
    pub local_score: f64,
    pub summary: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmCandidateUpdate {
    pub target_id: String,
    pub link_type: String,
    pub confidence: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct LinkDiscoveryService {
    notes_dir: PathBuf,
    queue_path: PathBuf,
    profiles: HashMap<String, LinkDiscoveryProfile>,
    stored_notes: HashMap<String, StoredDiscoveryNote>,
    queue: Vec<QueuedNote>,
    sweep_cursor: usize,
    last_run_at: Option<DateTime<Utc>>,
    current_note_id: Option<String>,
}

impl LinkDiscoveryService {
    pub fn new(data_path: PathBuf) -> Self {
        let base_dir = data_path.join("link_discovery");
        let notes_dir = base_dir.join("notes");
        let queue_path = base_dir.join("queue.json");
        let _ = std::fs::create_dir_all(&notes_dir);

        let mut service = Self {
            notes_dir,
            queue_path,
            profiles: HashMap::new(),
            stored_notes: HashMap::new(),
            queue: Vec::new(),
            sweep_cursor: 0,
            last_run_at: None,
            current_note_id: None,
        };
        service.load_from_disk();
        service
    }

    pub fn bootstrap(&mut self, notes: &[Note]) {
        let actual_ids: HashSet<String> = notes.iter().map(|note| note.id.clone()).collect();
        let title_to_id = build_reference_index(notes);

        for note in notes {
            let previous = self.stored_notes.get(&note.id).cloned();
            let last_discovered_at = previous.as_ref().and_then(|record| record.cached_at);
            let mut profile = build_profile(note, &title_to_id, last_discovered_at);
            let content_changed = previous
                .as_ref()
                .map(|record| record.profile.content_hash != profile.content_hash)
                .unwrap_or(true);

            let mut record = previous.unwrap_or_default();
            if content_changed {
                record.dismissed_target_ids.clear();
                record.links.clear();
                record.exploratory_links.clear();
                record.cached_at = None;
                record.is_stale = true;
            }
            if record.cached_at.is_none() {
                record.is_stale = true;
            }

            profile.is_stale = record.is_stale;
            profile.last_discovered_at = record.cached_at;
            record.profile = profile.clone();

            self.profiles.insert(note.id.clone(), profile.clone());
            self.stored_notes.insert(note.id.clone(), record.clone());
            self.persist_note_record(&note.id);

            if content_changed {
                self.enqueue(note.id.clone(), QueuePriority::Dirty);
            } else if record.is_stale {
                self.enqueue(note.id.clone(), QueuePriority::Stale);
            }
            if profile.existing_link_ids.is_empty() {
                self.enqueue(note.id.clone(), QueuePriority::Unlinked);
            }
        }

        let deleted_ids = self
            .stored_notes
            .keys()
            .filter(|note_id| !actual_ids.contains(*note_id))
            .cloned()
            .collect::<Vec<_>>();
        for note_id in deleted_ids {
            self.remove_note(&note_id);
        }

        self.persist_queue_state();
    }

    pub fn sync_note(&mut self, note: &Note) {
        let mut title_to_id = self
            .profiles
            .iter()
            .map(|(id, profile)| (profile.title.to_lowercase(), id.clone()))
            .collect::<HashMap<_, _>>();
        title_to_id.insert(note.title.to_lowercase(), note.id.clone());
        title_to_id.insert(note.relative_path.to_lowercase(), note.id.clone());
        for alias in &note.aliases {
            title_to_id.entry(alias.to_lowercase()).or_insert_with(|| note.id.clone());
        }

        let previous = self.stored_notes.get(&note.id).cloned();
        let last_discovered_at = previous.as_ref().and_then(|record| record.cached_at);
        let mut profile = build_profile(note, &title_to_id, last_discovered_at);
        let content_changed = previous
            .as_ref()
            .map(|record| record.profile.content_hash != profile.content_hash)
            .unwrap_or(true);

        let mut record = previous.unwrap_or_default();
        if content_changed {
            record.dismissed_target_ids.clear();
            record.links.clear();
            record.exploratory_links.clear();
            record.cached_at = None;
            record.is_stale = true;
        } else if record.cached_at.is_none() {
            record.is_stale = true;
        }

        profile.is_stale = record.is_stale;
        profile.last_discovered_at = record.cached_at;
        record.profile = profile.clone();

        self.profiles.insert(note.id.clone(), profile.clone());
        self.stored_notes.insert(note.id.clone(), record);
        self.persist_note_record(&note.id);

        self.enqueue(note.id.clone(), QueuePriority::Dirty);
        if profile.existing_link_ids.is_empty() {
            self.enqueue(note.id.clone(), QueuePriority::Unlinked);
        }
        self.persist_queue_state();
    }

    pub fn sync_notes(&mut self, notes: &[Note]) {
        for note in notes {
            self.sync_note(note);
        }
    }

    pub fn remove_note(&mut self, note_id: &str) {
        self.profiles.remove(note_id);
        self.stored_notes.remove(note_id);
        self.queue.retain(|queued| queued.note_id != note_id);
        self.current_note_id = self
            .current_note_id
            .clone()
            .filter(|current| current != note_id);

        let file_path = self.note_file_path(note_id);
        if file_path.exists() {
            let _ = std::fs::remove_file(&file_path);
        }
    }

    pub fn record_links_applied(&mut self, note_id: &str, target_ids: &[String]) {
        if let Some(record) = self.stored_notes.get_mut(note_id) {
            for target_id in target_ids {
                record.dismissed_target_ids.insert(target_id.clone());
            }
            record
                .links
                .retain(|candidate| !target_ids.contains(&candidate.target_id));
            record
                .exploratory_links
                .retain(|candidate| !target_ids.contains(&candidate.target_id));
            record.is_stale = true;
            if let Some(profile) = self.profiles.get_mut(note_id) {
                profile.is_stale = true;
            }
            self.enqueue(note_id.to_string(), QueuePriority::Dirty);
            self.persist_note_record(note_id);
        }

        for target_id in target_ids {
            if let Some(record) = self.stored_notes.get_mut(target_id) {
                record.is_stale = true;
                if let Some(profile) = self.profiles.get_mut(target_id) {
                    profile.is_stale = true;
                }
                self.enqueue(target_id.clone(), QueuePriority::Dirty);
                self.persist_note_record(target_id);
            }
        }

        self.persist_queue_state();
    }

    pub fn snapshot_for_note(&self, note_id: &str) -> Option<DiscoverySnapshot> {
        let source_profile = self.profiles.get(note_id)?.clone();
        let dismissed_target_ids = self
            .stored_notes
            .get(note_id)
            .map(|record| record.dismissed_target_ids.clone())
            .unwrap_or_default();

        Some(DiscoverySnapshot {
            source_profile,
            dismissed_target_ids,
        })
    }

    pub fn get_cached_response(
        &self,
        note_id: &str,
        max_links: usize,
    ) -> Option<DiscoverLinksResponse> {
        let record = self.stored_notes.get(note_id)?;
        if record.cached_at.is_none() || record.is_stale {
            return None;
        }
        Some(self.build_response_from_record(note_id, record, max_links, "cache"))
    }

    pub fn store_discovery_result(
        &mut self,
        note_id: &str,
        links: Vec<ZettelLinkCandidate>,
        exploratory_links: Vec<ZettelLinkCandidate>,
    ) -> Option<DiscoverLinksResponse> {
        let response_record = {
            let record = self.stored_notes.get_mut(note_id)?;
            let cached_at = Utc::now();

            record.links = links;
            record.exploratory_links = exploratory_links;
            record.cached_at = Some(cached_at);
            record.is_stale = false;
            record.profile.last_discovered_at = Some(cached_at);
            record.profile.is_stale = false;
            record.clone()
        };

        if let Some(profile) = self.profiles.get_mut(note_id) {
            profile.last_discovered_at = response_record.cached_at;
            profile.is_stale = false;
        }

        self.persist_note_record(note_id);
        Some(self.build_response_from_record(
            note_id,
            &response_record,
            MAX_LOCAL_CANDIDATES,
            "fresh",
        ))
    }

    pub fn list_queue_entries(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Vec<LinkSuggestionQueueEntry> {
        let include_all = status
            .map(|value| value.eq_ignore_ascii_case("all"))
            .unwrap_or(false);

        let mut entries = self
            .stored_notes
            .iter()
            .filter_map(|(note_id, record)| {
                let pending_count = record.links.len() + record.exploratory_links.len();
                if pending_count == 0 && !include_all {
                    return None;
                }

                let profile = self.profiles.get(note_id).unwrap_or(&record.profile);
                Some(LinkSuggestionQueueEntry {
                    note_id: note_id.clone(),
                    note_title: profile.title.clone(),
                    links: record.links.clone(),
                    exploratory_links: record.exploratory_links.clone(),
                    cached_at: record.cached_at,
                    is_stale: record.is_stale,
                    source: "cache".to_string(),
                    status: if pending_count > 0 {
                        "pending".to_string()
                    } else {
                        "empty".to_string()
                    },
                    priority: self
                        .queue
                        .iter()
                        .find(|queued| queued.note_id == *note_id)
                        .map(|queued| queued.priority.as_str().to_string())
                        .unwrap_or_default(),
                    pending_count,
                    updated_at: Some(profile.updated_at),
                })
            })
            .collect::<Vec<_>>();

        entries.sort_by(|a, b| {
            b.cached_at
                .cmp(&a.cached_at)
                .then_with(|| b.pending_count.cmp(&a.pending_count))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        entries.truncate(limit);
        entries
    }

    pub fn dismiss_suggestion(
        &mut self,
        note_id: &str,
        target_id: &str,
    ) -> DismissLinkSuggestionResponse {
        let mut removed = false;
        let mut remaining = 0;

        if let Some(record) = self.stored_notes.get_mut(note_id) {
            let before = record.links.len() + record.exploratory_links.len();
            record.dismissed_target_ids.insert(target_id.to_string());
            record
                .links
                .retain(|candidate| candidate.target_id != target_id);
            record
                .exploratory_links
                .retain(|candidate| candidate.target_id != target_id);
            remaining = record.links.len() + record.exploratory_links.len();
            removed = before != remaining;
            self.persist_note_record(note_id);
        }

        DismissLinkSuggestionResponse {
            note_id: note_id.to_string(),
            removed,
            remaining,
        }
    }

    pub fn status(&self, settings: &UserSettings) -> LinkDiscoveryStatus {
        let pending_notes = self
            .stored_notes
            .values()
            .filter(|record| !record.links.is_empty() || !record.exploratory_links.is_empty())
            .count();
        let pending_suggestions = self
            .stored_notes
            .values()
            .map(|record| record.links.len() + record.exploratory_links.len())
            .sum();
        let stale_notes = self
            .stored_notes
            .values()
            .filter(|record| record.is_stale)
            .count();

        LinkDiscoveryStatus {
            enabled: settings.background_link_discovery_enabled,
            llm_enabled: settings.background_link_discovery_llm_enabled,
            is_running: self.current_note_id.is_some(),
            queue_size: self.queue.len(),
            pending_notes,
            pending_suggestions,
            stale_notes,
            last_run_at: self.last_run_at,
            current_note_id: self.current_note_id.clone(),
            current_note_title: self
                .current_note_id
                .as_ref()
                .and_then(|note_id| self.profiles.get(note_id))
                .map(|profile| profile.title.clone()),
        }
    }

    pub fn next_background_job(
        &mut self,
        settings: &UserSettings,
    ) -> Option<BackgroundDiscoveryJob> {
        if !settings.background_link_discovery_enabled || self.current_note_id.is_some() {
            return None;
        }

        if self.queue.is_empty() {
            self.enqueue_next_sweep();
        }

        self.queue.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.enqueued_at.cmp(&b.enqueued_at))
        });

        let next = self.queue.first()?.clone();
        self.queue.remove(0);
        self.current_note_id = Some(next.note_id.clone());
        self.persist_queue_state();

        Some(BackgroundDiscoveryJob {
            note_id: next.note_id,
            mode: if settings.background_link_discovery_llm_enabled
                && next.priority.is_high_priority()
            {
                DiscoverMode::Llm
            } else {
                DiscoverMode::Algorithm
            },
            priority: next.priority.as_str().to_string(),
        })
    }

    pub fn complete_background_job(&mut self, note_id: &str, requeue_priority: Option<&str>) {
        if self.current_note_id.as_deref() == Some(note_id) {
            self.current_note_id = None;
        }
        self.last_run_at = Some(Utc::now());

        if let Some(priority) = requeue_priority.and_then(parse_queue_priority) {
            self.enqueue(note_id.to_string(), priority);
        }

        self.persist_queue_state();
    }

    pub fn mark_note_stale(&mut self, note_id: &str, priority: &str) {
        if let Some(record) = self.stored_notes.get_mut(note_id) {
            record.is_stale = true;
            if let Some(profile) = self.profiles.get_mut(note_id) {
                profile.is_stale = true;
            }
            self.enqueue(
                note_id.to_string(),
                parse_queue_priority(priority).unwrap_or(QueuePriority::Stale),
            );
            self.persist_note_record(note_id);
            self.persist_queue_state();
        }
    }

    fn build_response_from_record(
        &self,
        note_id: &str,
        record: &StoredDiscoveryNote,
        max_links: usize,
        source: &str,
    ) -> DiscoverLinksResponse {
        DiscoverLinksResponse {
            note_id: note_id.to_string(),
            links: record.links.iter().take(max_links).cloned().collect(),
            exploratory_links: record
                .exploratory_links
                .iter()
                .take(exploratory_limit(max_links))
                .cloned()
                .collect(),
            topic_hubs: self.build_topic_hub_candidates(&record.profile),
            cached_at: record.cached_at,
            is_stale: record.is_stale,
            source: source.to_string(),
        }
    }

    fn build_topic_hub_candidates(&self, profile: &LinkDiscoveryProfile) -> Vec<TopicHubCandidate> {
        let mut topic_hubs = if profile.is_topic_hub {
            vec![TopicHubCandidate {
                hub_id: profile.note_id.clone(),
                hub_title: profile.title.clone(),
                topic_key: profile
                    .topic_key
                    .clone()
                    .unwrap_or_else(|| profile.title.to_lowercase()),
                membership_source: "hub".to_string(),
            }]
        } else {
            profile
                .topic_hub_ids
                .iter()
                .filter_map(|hub_id| {
                    let hub_profile = self.profiles.get(hub_id)?;
                    Some(TopicHubCandidate {
                        hub_id: hub_id.clone(),
                        hub_title: hub_profile.title.clone(),
                        topic_key: hub_profile
                            .topic_key
                            .clone()
                            .or_else(|| profile.topic_key.clone())
                            .unwrap_or_else(|| hub_profile.title.to_lowercase()),
                        membership_source: "auto".to_string(),
                    })
                })
                .collect::<Vec<_>>()
        };

        topic_hubs.sort_by(|left, right| {
            left.hub_title
                .cmp(&right.hub_title)
                .then_with(|| left.hub_id.cmp(&right.hub_id))
        });
        topic_hubs.dedup_by(|left, right| left.hub_id == right.hub_id);
        topic_hubs
    }

    fn enqueue(&mut self, note_id: String, priority: QueuePriority) {
        if self.current_note_id.as_deref() == Some(note_id.as_str()) {
            return;
        }

        if let Some(existing) = self
            .queue
            .iter_mut()
            .find(|queued| queued.note_id == note_id)
        {
            if priority < existing.priority {
                existing.priority = priority;
            }
            return;
        }

        self.queue.push(QueuedNote {
            note_id,
            priority,
            enqueued_at: Utc::now(),
        });
    }

    fn enqueue_next_sweep(&mut self) {
        let mut note_ids = self.profiles.keys().cloned().collect::<Vec<_>>();
        note_ids.sort();
        if note_ids.is_empty() {
            return;
        }

        let mut attempts = 0;
        while attempts < note_ids.len() {
            let note_id = note_ids[self.sweep_cursor % note_ids.len()].clone();
            self.sweep_cursor = (self.sweep_cursor + 1) % note_ids.len();
            attempts += 1;

            let already_queued = self.queue.iter().any(|queued| queued.note_id == note_id);
            let is_current = self.current_note_id.as_deref() == Some(note_id.as_str());
            if !already_queued && !is_current {
                self.enqueue(note_id, QueuePriority::Sweep);
                break;
            }
        }
    }

    fn load_from_disk(&mut self) {
        if self.notes_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&self.notes_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                        continue;
                    }

                    if let Ok(contents) = std::fs::read_to_string(&path) {
                        if let Ok(record) = serde_json::from_str::<StoredDiscoveryNote>(&contents) {
                            let note_id = record.profile.note_id.clone();
                            self.profiles
                                .insert(note_id.clone(), record.profile.clone());
                            self.stored_notes.insert(note_id, record);
                        }
                    }
                }
            }
        }

        if self.queue_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&self.queue_path) {
                if let Ok(state) = serde_json::from_str::<PersistedQueueState>(&contents) {
                    self.queue = state.queue;
                    self.sweep_cursor = state.sweep_cursor;
                    self.last_run_at = state.last_run_at;
                }
            }
        }
    }

    fn persist_note_record(&self, note_id: &str) {
        if let Some(record) = self.stored_notes.get(note_id) {
            let path = self.note_file_path(note_id);
            if let Ok(contents) = serde_json::to_string_pretty(record) {
                let _ = std::fs::write(path, contents);
            }
        }
    }

    fn persist_queue_state(&self) {
        let state = PersistedQueueState {
            queue: self.queue.clone(),
            sweep_cursor: self.sweep_cursor,
            last_run_at: self.last_run_at,
        };

        if let Some(parent) = self.queue_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if let Ok(contents) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(&self.queue_path, contents);
        }
    }

    fn note_file_path(&self, note_id: &str) -> PathBuf {
        self.notes_dir.join(format!("{}.json", note_id))
    }
}

fn parse_queue_priority(value: &str) -> Option<QueuePriority> {
    match value {
        "dirty" => Some(QueuePriority::Dirty),
        "unlinked" => Some(QueuePriority::Unlinked),
        "stale" => Some(QueuePriority::Stale),
        "sweep" => Some(QueuePriority::Sweep),
        _ => None,
    }
}

pub fn rank_local_candidates(
    mut signals: Vec<LocalCandidateSignals>,
    limit: usize,
) -> Vec<RankedLinkCandidate> {
    for signal in &mut signals {
        signal.note_score = signal.note_score.clamp(0.0, 1.0);
        signal.chunk_score = signal.chunk_score.clamp(0.0, 1.0);
        signal.tag_overlap_ratio = signal.tag_overlap_ratio.clamp(0.0, 1.0);
        signal.key_term_cosine = signal.key_term_cosine.clamp(0.0, 1.0);
        signal.graph_proximity = signal.graph_proximity.clamp(0.0, 1.0);
    }

    let mut ranked = signals
        .into_iter()
        .map(|signal| {
            let local_score = (signal.note_score * 0.35)
                + (signal.chunk_score * 0.20)
                + (signal.tag_overlap_ratio * 0.15)
                + (signal.key_term_cosine * 0.20)
                + (signal.graph_proximity * 0.10);

            let link_type = if signal.graph_proximity >= 0.45 && signal.note_score >= 0.65 {
                "expands"
            } else {
                "related"
            };
            let confidence = (0.35 + (local_score * 0.60)).min(0.96);

            let reason = format!(
                "Local match: note {:.2}, chunk {:.2}, tags {}, terms {:.2}, graph {:.2}",
                signal.note_score,
                signal.chunk_score,
                signal.tag_overlap,
                signal.key_term_cosine,
                signal.graph_proximity
            );

            RankedLinkCandidate {
                candidate: ZettelLinkCandidate {
                    target_id: signal.target_id,
                    target_title: signal.target_title,
                    link_type: link_type.to_string(),
                    confidence: round_confidence(confidence),
                    reason,
                },
                local_score,
                summary: signal.summary,
                snippet: signal.snippet,
            }
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|a, b| {
        b.local_score
            .partial_cmp(&a.local_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                b.candidate
                    .confidence
                    .partial_cmp(&a.candidate.confidence)
                    .unwrap_or(Ordering::Equal)
            })
    });
    ranked.truncate(limit);
    ranked
}

pub fn sample_exploratory_candidates(
    source_profile: &LinkDiscoveryProfile,
    all_profiles: &HashMap<String, LinkDiscoveryProfile>,
    excluded_ids: &HashSet<String>,
    limit: usize,
) -> Vec<RankedLinkCandidate> {
    let source_tags = source_profile
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<HashSet<_>>();
    let source_terms = source_profile
        .key_terms
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    let mut plausible_pool = all_profiles
        .values()
        .filter(|profile| {
            !profile.is_topic_hub
                && profile.note_id != source_profile.note_id
                && !excluded_ids.contains(&profile.note_id)
        })
        .filter_map(|profile| {
            let term_cosine = cosine_similarity(&source_profile.term_vector, &profile.term_vector);
            let candidate_tags = profile
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect::<HashSet<_>>();
            let tag_overlap = source_tags.intersection(&candidate_tags).count();
            let title_overlap = title_token_overlap(&source_profile.title, &profile.title) as f64;
            let shared_key_terms = source_terms
                .intersection(&profile.key_terms.iter().cloned().collect::<HashSet<_>>())
                .count();
            let weak_signal = term_cosine.max((tag_overlap as f64 * 0.18) + (title_overlap * 0.12));
            let is_unlinked = profile.existing_link_ids.is_empty();

            if weak_signal < 0.04 && !is_unlinked && shared_key_terms == 0 {
                return None;
            }

            let novelty_score = ((1.0 - term_cosine).clamp(0.0, 1.0) * 0.25)
                + (if is_unlinked { 0.45 } else { 0.10 })
                + (if tag_overlap == 0 { 0.20 } else { 0.05 })
                + (if shared_key_terms == 0 { 0.10 } else { 0.0 });
            let exploration_score = (weak_signal * 0.45) + novelty_score;
            let seed = stable_hash_u64(&format!(
                "{}:{}:{}",
                source_profile.note_id, profile.note_id, source_profile.content_hash
            ));

            Some((seed, exploration_score, weak_signal, is_unlinked, profile))
        })
        .collect::<Vec<_>>();

    plausible_pool.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    plausible_pool.truncate(limit.saturating_mul(4).max(limit));
    plausible_pool.sort_by(|a, b| a.0.cmp(&b.0));

    plausible_pool
        .into_iter()
        .take(limit)
        .map(
            |(_, exploration_score, weak_signal, is_unlinked, profile)| {
                let confidence =
                    (0.20 + (exploration_score * 0.28) + if is_unlinked { 0.06 } else { 0.0 })
                        .min(0.72);

                RankedLinkCandidate {
                    candidate: ZettelLinkCandidate {
                        target_id: profile.note_id.clone(),
                        target_title: profile.title.clone(),
                        link_type: "related".to_string(),
                        confidence: round_confidence(confidence),
                        reason: format!(
                            "Exploratory match: weak signal {:.2}, novelty {:.2}{}",
                            weak_signal,
                            exploration_score,
                            if is_unlinked { ", unlinked note" } else { "" }
                        ),
                    },
                    local_score: exploration_score,
                    summary: profile.summary.clone(),
                    snippet: profile.summary.clone(),
                }
            },
        )
        .collect()
}

pub fn parse_llm_rerank_response(response: &str) -> Vec<LlmCandidateUpdate> {
    let json_slice = match JSON_ARRAY_RE.find(response) {
        Some(matched) => matched.as_str(),
        None => return Vec::new(),
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(json_slice) {
        Ok(values) => values,
        Err(_) => return Vec::new(),
    };

    parsed
        .into_iter()
        .filter_map(|item| {
            let target_id = item
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_string();
            let link_type = item
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("related")
                .to_string();
            let confidence = item
                .get("confidence")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.75)
                .clamp(0.0, 1.0);
            let reason = item
                .get("reason")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .chars()
                .take(100)
                .collect::<String>();

            Some(LlmCandidateUpdate {
                target_id,
                link_type,
                confidence: round_confidence(confidence),
                reason,
            })
        })
        .collect()
}

pub fn apply_llm_updates(
    candidates: Vec<RankedLinkCandidate>,
    updates: &[LlmCandidateUpdate],
) -> Vec<RankedLinkCandidate> {
    let update_map = updates
        .iter()
        .map(|update| (update.target_id.clone(), update))
        .collect::<HashMap<_, _>>();

    let mut merged = candidates
        .into_iter()
        .map(|mut candidate| {
            if let Some(update) = update_map.get(&candidate.candidate.target_id) {
                candidate.candidate.link_type = update.link_type.clone();
                candidate.candidate.confidence = update.confidence;
                if !update.reason.is_empty() {
                    candidate.candidate.reason = update.reason.clone();
                }
                candidate.local_score = (candidate.local_score * 0.55) + (update.confidence * 0.45);
            }
            candidate
        })
        .collect::<Vec<_>>();

    merged.sort_by(|a, b| {
        b.candidate
            .confidence
            .partial_cmp(&a.candidate.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                b.local_score
                    .partial_cmp(&a.local_score)
                    .unwrap_or(Ordering::Equal)
            })
    });
    merged
}

pub fn build_profile(
    note: &Note,
    title_to_id: &HashMap<String, String>,
    last_discovered_at: Option<DateTime<Utc>>,
) -> LinkDiscoveryProfile {
    let summary = extract_summary(&note.content);
    let clean_content = clean_markdown(&note.content);
    let compact_content = truncate_text(clean_content.trim(), PROFILE_VECTOR_TEXT_LIMIT);
    let vector_source = format!("{} {} {}", note.title, summary, compact_content);
    let provider = TfIdfProvider::new();
    let term_vector = provider.encode(&vector_source).terms;
    let key_terms = extract_key_terms(&vector_source)
        .into_iter()
        .collect::<Vec<_>>();
    let existing_link_ids = note
        .parsed_links
        .iter()
        .filter_map(|parsed_link| {
            parsed_link
                .target_path
                .as_ref()
                .and_then(|target_path| title_to_id.get(&target_path.to_lowercase()))
                .or_else(|| title_to_id.get(&parsed_link.target_title.to_lowercase()))
        })
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    LinkDiscoveryProfile {
        note_id: note.id.clone(),
        title: note.title.clone(),
        tags: note.tags.clone(),
        topic_key: note.topic_key(),
        topic_hub_ids: note.topic_hub_ids(),
        is_topic_hub: note.is_topic_hub(),
        summary,
        key_terms,
        term_vector,
        existing_link_ids,
        content_hash: stable_hash_string(&format!(
            "{}\n{}\n{}",
            note.title,
            note.content,
            note.tags.join(",")
        )),
        updated_at: note.updated_at,
        last_discovered_at,
        is_stale: last_discovered_at.is_none(),
    }
}

fn build_reference_index(notes: &[Note]) -> HashMap<String, String> {
    let mut refs = HashMap::new();
    for note in notes {
        refs.entry(note.title.to_lowercase())
            .or_insert_with(|| note.id.clone());
        refs.entry(note.relative_path.to_lowercase())
            .or_insert_with(|| note.id.clone());
        for alias in &note.aliases {
            refs.entry(alias.to_lowercase())
                .or_insert_with(|| note.id.clone());
        }
    }
    refs
}

pub fn cosine_similarity(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    for (term, weight_a) in a {
        norm_a += weight_a * weight_a;
        if let Some(weight_b) = b.get(term) {
            dot_product += weight_a * weight_b;
        }
    }

    let norm_b = b.values().map(|weight| weight * weight).sum::<f64>();
    let magnitude = (norm_a * norm_b).sqrt();
    if magnitude < 1e-10 {
        0.0
    } else {
        dot_product / magnitude
    }
}

pub fn title_token_overlap(left: &str, right: &str) -> usize {
    let left_tokens = tokenize_simple(left).into_iter().collect::<HashSet<_>>();
    let right_tokens = tokenize_simple(right).into_iter().collect::<HashSet<_>>();
    left_tokens.intersection(&right_tokens).count()
}

pub fn clean_markdown(content: &str) -> String {
    let clean = FENCED_CODE_RE.replace_all(content, "");
    let clean = INLINE_CODE_RE.replace_all(&clean, "");
    WIKILINK_RE.replace_all(&clean, "").to_string()
}

pub fn extract_summary(content: &str) -> String {
    let tldr = content
        .lines()
        .skip_while(|line| !line.contains("## TL;DR"))
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect::<Vec<_>>()
        .join(" ");
    if !tldr.trim().is_empty() {
        return truncate_text(tldr.trim(), 280);
    }

    let clean = clean_markdown(content);
    let first_paragraph = clean
        .split("\n\n")
        .map(str::trim)
        .find(|paragraph| !paragraph.is_empty() && !paragraph.starts_with('#'))
        .unwrap_or("");

    truncate_text(first_paragraph, 280)
}

pub fn extract_key_terms(content: &str) -> HashSet<String> {
    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let clean = clean_markdown(content);
    let mut terms = HashSet::new();

    let config = YakeConfig {
        max_ngram_size: 2,
        top_k: 15,
        ..YakeConfig::default()
    };
    for keyphrase in yake::extract_keyphrases(&clean, &config) {
        terms.insert(keyphrase.text.to_lowercase());
    }

    for matched in PROPER_NOUN_RE.find_iter(&clean) {
        let term = matched.as_str().to_lowercase();
        if term.len() >= 3 && !stopwords.contains(term.as_str()) {
            terms.insert(term);
        }
    }

    for matched in ACRONYM_RE.find_iter(&clean) {
        let term = matched.as_str().to_lowercase();
        if term.len() >= 2 {
            terms.insert(term);
        }
    }

    terms
}

fn truncate_text(content: &str, max_chars: usize) -> String {
    content.chars().take(max_chars).collect::<String>()
}

fn round_confidence(confidence: f64) -> f64 {
    (confidence * 100.0).round() / 100.0
}

fn tokenize_simple(content: &str) -> Vec<String> {
    content
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_lowercase())
        .collect()
}

fn stable_hash_string(value: &str) -> String {
    stable_hash_u64(value).to_string()
}

fn stable_hash_u64<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn exploratory_limit(max_links: usize) -> usize {
    max_links.clamp(5, 8)
}

#[cfg(feature = "tauri-app")]
pub async fn discover_for_note(
    state: &AppState,
    note_id: &str,
    mode: DiscoverMode,
    max_links: usize,
    allow_cache: bool,
) -> Result<DiscoverLinksResponse, String> {
    let total_started_at = Instant::now();

    if mode == DiscoverMode::Manual {
        return Ok(DiscoverLinksResponse {
            note_id: note_id.to_string(),
            links: Vec::new(),
            exploratory_links: Vec::new(),
            topic_hubs: Vec::new(),
            cached_at: None,
            is_stale: false,
            source: "fresh".to_string(),
        });
    }

    if allow_cache {
        let discovery = state.link_discovery.read().await;
        if let Some(cached) = discovery.get_cached_response(note_id, max_links) {
            log::debug!(
                "Discover links {:?} for '{}' served from cache",
                mode,
                note_id
            );
            return Ok(cached);
        }
    }

    let snapshot = {
        let discovery = state.link_discovery.read().await;
        discovery
            .snapshot_for_note(note_id)
            .ok_or_else(|| format!("Note not found in link discovery cache: {}", note_id))?
    };

    let query = build_discovery_query(&snapshot.source_profile);

    let note_results = {
        let search = state.search_service.read().await;
        let graph = state.graph_index.read().await;
        let priority = state.priority_service.read().await;
        let retrieval = state.retrieval_service.read().await;
        retrieval
            .retrieve(
                &search,
                &graph,
                &priority,
                &query,
                MAX_LOCAL_CANDIDATES,
                &[note_id.to_string()],
            )
            .unwrap_or_default()
    };

    let chunk_results = {
        let chunks = state.chunk_index.read().await;
        let graph = state.graph_index.read().await;
        let priority = state.priority_service.read().await;
        let retrieval = state.retrieval_service.read().await;
        retrieval
            .retrieve_chunks(
                &chunks,
                &graph,
                &priority,
                &query,
                DEFAULT_CHUNK_LIMIT * 32,
                &[note_id.to_string()],
            )
            .unwrap_or_default()
    };

    let second_hop_counts = {
        let graph = state.graph_index.read().await;
        build_second_hop_counts(&graph, &snapshot.source_profile)
    };

    let topic_relation_scores = {
        let graph = state.graph_index.read().await;
        note_results
            .iter()
            .map(|result| result.note.id.clone())
            .chain(
                chunk_results
                    .iter()
                    .map(|chunk| chunk.parent_note_id.clone()),
            )
            .chain(second_hop_counts.keys().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|candidate_id| {
                let score = graph.topic_relation_score(note_id, &candidate_id);
                (candidate_id, score)
            })
            .collect::<HashMap<_, _>>()
    };

    let ranking_started_at = Instant::now();
    let (mut ranked_links, mut exploratory_ranked) = {
        let discovery = state.link_discovery.read().await;
        let ranked_links = build_local_ranked_candidates(
            &snapshot.source_profile,
            &snapshot.dismissed_target_ids,
            &discovery.profiles,
            &note_results,
            &chunk_results,
            &second_hop_counts,
            &topic_relation_scores,
        );

        let mut excluded_ids = snapshot.dismissed_target_ids.clone();
        excluded_ids.insert(snapshot.source_profile.note_id.clone());
        excluded_ids.extend(snapshot.source_profile.existing_link_ids.iter().cloned());
        excluded_ids.extend(
            ranked_links
                .iter()
                .take(MAX_LOCAL_CANDIDATES)
                .map(|candidate| candidate.candidate.target_id.clone()),
        );

        let exploratory_ranked = sample_exploratory_candidates(
            &snapshot.source_profile,
            &discovery.profiles,
            &excluded_ids,
            exploratory_limit(max_links),
        );

        (ranked_links, exploratory_ranked)
    };

    if mode.include_llm() {
        ranked_links =
            rerank_with_llm(state, &snapshot.source_profile, ranked_links, "strong", 8).await;
    }
    if mode.include_llm() {
        exploratory_ranked = rerank_with_llm(
            state,
            &snapshot.source_profile,
            exploratory_ranked,
            "exploratory",
            3,
        )
        .await;
    }

    log::debug!(
        "Discover links {:?} for '{}' completed in {} ms (ranking {} ms, notes {}, chunks {})",
        mode,
        note_id,
        total_started_at.elapsed().as_millis(),
        ranking_started_at.elapsed().as_millis(),
        note_results.len(),
        chunk_results.len()
    );

    let links = ranked_links
        .iter()
        .take(max_links)
        .map(|candidate| candidate.candidate.clone())
        .collect::<Vec<_>>();
    let exploratory_links = exploratory_ranked
        .iter()
        .take(exploratory_limit(max_links))
        .map(|candidate| candidate.candidate.clone())
        .collect::<Vec<_>>();

    let mut discovery = state.link_discovery.write().await;
    let stored = discovery
        .store_discovery_result(note_id, links.clone(), exploratory_links.clone())
        .unwrap_or(DiscoverLinksResponse {
            note_id: note_id.to_string(),
            links: links.clone(),
            exploratory_links: exploratory_links.clone(),
            topic_hubs: Vec::new(),
            cached_at: Some(Utc::now()),
            is_stale: false,
            source: "fresh".to_string(),
        });

    Ok(DiscoverLinksResponse {
        note_id: stored.note_id,
        links: stored.links.into_iter().take(max_links).collect(),
        exploratory_links: stored
            .exploratory_links
            .into_iter()
            .take(exploratory_limit(max_links))
            .collect(),
        topic_hubs: stored.topic_hubs,
        cached_at: stored.cached_at,
        is_stale: stored.is_stale,
        source: "fresh".to_string(),
    })
}

fn build_local_ranked_candidates(
    source_profile: &LinkDiscoveryProfile,
    dismissed_target_ids: &HashSet<String>,
    all_profiles: &HashMap<String, LinkDiscoveryProfile>,
    note_results: &[RetrievalResult],
    chunk_results: &[ChunkResult],
    second_hop_counts: &HashMap<String, usize>,
    topic_relation_scores: &HashMap<String, f64>,
) -> Vec<RankedLinkCandidate> {
    let source_tags = source_profile
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<HashSet<_>>();
    let excluded_ids = source_profile
        .existing_link_ids
        .iter()
        .cloned()
        .chain(std::iter::once(source_profile.note_id.clone()))
        .chain(dismissed_target_ids.iter().cloned())
        .collect::<HashSet<_>>();

    let max_note_score = note_results
        .iter()
        .map(|result| result.score)
        .fold(0.0_f32, f32::max)
        .max(0.001);
    let max_chunk_score = chunk_results
        .iter()
        .map(|result| result.search_score)
        .fold(0.0_f32, f32::max)
        .max(0.001);

    let mut signals_map: HashMap<String, LocalCandidateSignals> = HashMap::new();

    for result in note_results {
        if excluded_ids.contains(&result.note.id) {
            continue;
        }
        if let Some(profile) = all_profiles.get(&result.note.id) {
            if profile.is_topic_hub {
                continue;
            }
            let entry = signals_map
                .entry(result.note.id.clone())
                .or_insert_with(|| base_signals(profile));
            entry.note_score = (result.score / max_note_score) as f64;
            if entry.snippet.is_empty() {
                entry.snippet = result.snippet.clone();
            }
        }
    }

    for chunk in chunk_results {
        if excluded_ids.contains(&chunk.parent_note_id) {
            continue;
        }
        if let Some(profile) = all_profiles.get(&chunk.parent_note_id) {
            if profile.is_topic_hub {
                continue;
            }
            let entry = signals_map
                .entry(chunk.parent_note_id.clone())
                .or_insert_with(|| base_signals(profile));
            let normalized = (chunk.search_score / max_chunk_score) as f64;
            if normalized > entry.chunk_score {
                entry.chunk_score = normalized;
                entry.snippet = truncate_text(chunk.text.trim(), 180);
            }
        }
    }

    let max_second_hop = second_hop_counts
        .values()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    for (candidate_id, count) in second_hop_counts {
        if excluded_ids.contains(candidate_id) {
            continue;
        }
        if let Some(profile) = all_profiles.get(candidate_id) {
            if profile.is_topic_hub {
                continue;
            }
            let entry = signals_map
                .entry(candidate_id.clone())
                .or_insert_with(|| base_signals(profile));
            let second_hop_score = (*count as f64 / max_second_hop).clamp(0.0, 1.0);
            entry.graph_proximity = entry.graph_proximity.max(second_hop_score);
        }
    }

    let source_link_set = source_profile
        .existing_link_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    for (candidate_id, entry) in &mut signals_map {
        if let Some(profile) = all_profiles.get(candidate_id) {
            if profile.is_topic_hub {
                continue;
            }
            let candidate_tags = profile
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect::<HashSet<_>>();
            let tag_overlap = source_tags.intersection(&candidate_tags).count();
            let tag_union = source_tags.union(&candidate_tags).count().max(1);
            let shared_neighbors = profile
                .existing_link_ids
                .iter()
                .filter(|linked_id| source_link_set.contains(*linked_id))
                .count();

            entry.tag_overlap = tag_overlap;
            entry.tag_overlap_ratio = tag_overlap as f64 / tag_union as f64;
            entry.key_term_cosine =
                cosine_similarity(&source_profile.term_vector, &profile.term_vector);
            if !source_link_set.is_empty() {
                let overlap_score =
                    (shared_neighbors as f64 / source_link_set.len() as f64).clamp(0.0, 1.0);
                entry.graph_proximity = entry.graph_proximity.max(overlap_score);
            }
            if let Some(topic_score) = topic_relation_scores.get(candidate_id) {
                entry.graph_proximity = entry.graph_proximity.max(*topic_score);
            }
        }
    }

    let signals = signals_map
        .into_values()
        .filter(|signal| {
            signal.note_score > 0.0
                || signal.chunk_score > 0.0
                || signal.graph_proximity > 0.0
                || signal.key_term_cosine > 0.08
                || signal.tag_overlap > 0
        })
        .collect::<Vec<_>>();

    rank_local_candidates(signals, MAX_LOCAL_CANDIDATES)
}

fn base_signals(profile: &LinkDiscoveryProfile) -> LocalCandidateSignals {
    LocalCandidateSignals {
        target_id: profile.note_id.clone(),
        target_title: profile.title.clone(),
        summary: profile.summary.clone(),
        snippet: profile.summary.clone(),
        ..Default::default()
    }
}

fn build_discovery_query(profile: &LinkDiscoveryProfile) -> String {
    let mut terms = vec![profile.title.clone()];
    if !profile.summary.is_empty() {
        terms.push(profile.summary.clone());
    }
    terms.extend(profile.key_terms.iter().take(8).cloned());
    terms.join(" ")
}

fn build_second_hop_counts(
    graph: &crate::services::graph_index::GraphIndex,
    source_profile: &LinkDiscoveryProfile,
) -> HashMap<String, usize> {
    let direct_links = source_profile
        .existing_link_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut counts = HashMap::new();

    for linked_id in &source_profile.existing_link_ids {
        for neighbor in graph.get_neighbors(linked_id) {
            let candidate_id = neighbor.note.id;
            if candidate_id == source_profile.note_id || direct_links.contains(&candidate_id) {
                continue;
            }
            *counts.entry(candidate_id).or_insert(0) += 1;
        }
    }

    counts
}

#[cfg(feature = "tauri-app")]
async fn rerank_with_llm(
    state: &AppState,
    source_profile: &LinkDiscoveryProfile,
    candidates: Vec<RankedLinkCandidate>,
    bucket: &str,
    limit: usize,
) -> Vec<RankedLinkCandidate> {
    if candidates.is_empty() {
        return candidates;
    }

    let is_configured = {
        let openrouter = state.openrouter.read().await;
        openrouter.is_configured()
    };
    if !is_configured {
        return candidates;
    }

    let shortlist = candidates.iter().take(limit).cloned().collect::<Vec<_>>();
    let candidate_context = shortlist
        .iter()
        .map(|candidate| {
            format!(
                "- id: {}\n  title: {}\n  summary: {}\n  evidence: score {:.2}; {}\n  snippet: {}",
                candidate.candidate.target_id,
                candidate.candidate.target_title,
                candidate.summary,
                candidate.local_score,
                candidate.candidate.reason,
                truncate_text(candidate.snippet.trim(), 180)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "You are reviewing potential knowledge-graph links for a note.\n\
SOURCE NOTE:\n- title: {}\n- summary: {}\n\n\
Evaluate the following {} candidates. Select only the candidates worth keeping.\n\
Return ONLY a JSON array with objects:\n\
[{{\"id\":\"candidate-id\",\"type\":\"related\",\"confidence\":0.0,\"reason\":\"brief reason\"}}]\n\
Use confidence from 0.0 to 1.0. Types must be one of: related, supports, contradicts, expands, questions, answers, example, part_of.\n\n\
CANDIDATES:\n{}",
        source_profile.title,
        source_profile.summary,
        bucket,
        candidate_context
    );

    let model = {
        let settings = state.settings_service.read().await;
        settings.get().llm_model.clone()
    };
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let response = {
        let openrouter = state.openrouter.read().await;
        match openrouter
            .chat(&model, messages, None, Some(0.1), Some(900), false, 5)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                log::warn!("LLM link discovery rerank failed: {}", error);
                return candidates;
            }
        }
    };

    let updates = parse_llm_rerank_response(&response);
    if updates.is_empty() {
        return candidates;
    }

    let updated_shortlist = apply_llm_updates(shortlist, &updates);
    let updated_map = updated_shortlist
        .into_iter()
        .map(|candidate| (candidate.candidate.target_id.clone(), candidate))
        .collect::<HashMap<_, _>>();

    let mut merged = candidates
        .into_iter()
        .map(|candidate| {
            updated_map
                .get(&candidate.candidate.target_id)
                .cloned()
                .unwrap_or(candidate)
        })
        .collect::<Vec<_>>();
    merged.sort_by(|a, b| {
        b.candidate
            .confidence
            .partial_cmp(&a.candidate.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                b.local_score
                    .partial_cmp(&a.local_score)
                    .unwrap_or(Ordering::Equal)
            })
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteStatus, ParsedLink, RelationType};
    use std::collections::HashMap;

    fn make_note(id: &str, title: &str, content: &str, tags: &[&str], links: &[&str]) -> Note {
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: links.iter().map(|link| link.to_string()).collect(),
            parsed_links: links
                .iter()
                .map(|link| ParsedLink {
                    target_title: (*link).to_string(),
                    target_path: None,
                    relation: RelationType::Related,
                })
                .collect(),
            properties: HashMap::new(),
        }
    }

    #[test]
    fn build_profile_extracts_summary_terms_and_links() {
        let note = make_note(
            "n1",
            "Machine Learning",
            "## TL;DR\nMachine learning finds structure in data.\n\nMore details.",
            &["ai", "ml"],
            &["Deep Learning"],
        );
        let title_to_id = HashMap::from([
            ("machine learning".to_string(), "n1".to_string()),
            ("deep learning".to_string(), "n2".to_string()),
        ]);

        let profile = build_profile(&note, &title_to_id, None);

        assert_eq!(profile.note_id, "n1");
        assert!(profile.summary.contains("Machine learning"));
        assert!(profile.term_vector.contains_key("machine"));
        assert_eq!(profile.existing_link_ids, vec!["n2".to_string()]);
    }

    #[test]
    fn rank_local_candidates_prefers_stronger_signals() {
        let ranked = rank_local_candidates(
            vec![
                LocalCandidateSignals {
                    target_id: "a".to_string(),
                    target_title: "A".to_string(),
                    note_score: 0.8,
                    chunk_score: 0.7,
                    tag_overlap: 2,
                    tag_overlap_ratio: 0.8,
                    key_term_cosine: 0.7,
                    graph_proximity: 0.2,
                    ..Default::default()
                },
                LocalCandidateSignals {
                    target_id: "b".to_string(),
                    target_title: "B".to_string(),
                    note_score: 0.4,
                    chunk_score: 0.1,
                    key_term_cosine: 0.1,
                    ..Default::default()
                },
            ],
            10,
        );

        assert_eq!(ranked[0].candidate.target_id, "a");
        assert!(ranked[0].candidate.confidence > ranked[1].candidate.confidence);
    }

    #[test]
    fn exploratory_sampling_is_deterministic() {
        let source = LinkDiscoveryProfile {
            note_id: "source".to_string(),
            title: "Source".to_string(),
            tags: vec!["ai".to_string()],
            topic_key: None,
            topic_hub_ids: Vec::new(),
            is_topic_hub: false,
            summary: "About machine learning".to_string(),
            key_terms: vec!["machine learning".to_string()],
            term_vector: HashMap::from([
                ("machine".to_string(), 1.0),
                ("learning".to_string(), 1.0),
            ]),
            existing_link_ids: Vec::new(),
            content_hash: "123".to_string(),
            updated_at: Utc::now(),
            last_discovered_at: None,
            is_stale: true,
        };
        let candidates = HashMap::from([
            (
                "a".to_string(),
                LinkDiscoveryProfile {
                    note_id: "a".to_string(),
                    title: "A".to_string(),
                    tags: vec!["ai".to_string()],
                    topic_key: None,
                    topic_hub_ids: Vec::new(),
                    is_topic_hub: false,
                    summary: "Machine learning systems".to_string(),
                    key_terms: vec!["machine learning".to_string()],
                    term_vector: HashMap::from([("machine".to_string(), 1.0)]),
                    existing_link_ids: Vec::new(),
                    content_hash: "a".to_string(),
                    updated_at: Utc::now(),
                    last_discovered_at: None,
                    is_stale: true,
                },
            ),
            (
                "b".to_string(),
                LinkDiscoveryProfile {
                    note_id: "b".to_string(),
                    title: "B".to_string(),
                    tags: vec!["research".to_string()],
                    topic_key: None,
                    topic_hub_ids: Vec::new(),
                    is_topic_hub: false,
                    summary: "Model evaluation".to_string(),
                    key_terms: vec!["model evaluation".to_string()],
                    term_vector: HashMap::from([("model".to_string(), 1.0)]),
                    existing_link_ids: vec!["n9".to_string()],
                    content_hash: "b".to_string(),
                    updated_at: Utc::now(),
                    last_discovered_at: None,
                    is_stale: true,
                },
            ),
        ]);

        let first = sample_exploratory_candidates(&source, &candidates, &HashSet::new(), 2);
        let second = sample_exploratory_candidates(&source, &candidates, &HashSet::new(), 2);

        assert_eq!(
            first
                .iter()
                .map(|candidate| candidate.candidate.target_id.clone())
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|candidate| candidate.candidate.target_id.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn queue_prioritizes_dirty_before_sweep() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut service = LinkDiscoveryService::new(temp_dir.path().to_path_buf());
        service.enqueue("note-b".to_string(), QueuePriority::Sweep);
        service.enqueue("note-a".to_string(), QueuePriority::Dirty);

        let job = service
            .next_background_job(&UserSettings::default())
            .expect("job should exist");

        assert_eq!(job.note_id, "note-a");
        assert_eq!(job.priority, "dirty");
    }

    #[test]
    fn content_change_clears_dismissals_and_marks_stale() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut service = LinkDiscoveryService::new(temp_dir.path().to_path_buf());
        let note = make_note("n1", "Source", "Initial content", &["ai"], &[]);
        service.bootstrap(&[note.clone()]);
        service.dismiss_suggestion("n1", "target-1");

        let updated = make_note("n1", "Source", "Updated content", &["ai"], &[]);
        service.sync_note(&updated);

        let snapshot = service.snapshot_for_note("n1").expect("snapshot");
        assert!(snapshot.dismissed_target_ids.is_empty());
        assert!(service.stored_notes.get("n1").expect("record").is_stale);
    }

    #[test]
    fn parse_llm_response_reads_json_updates() {
        let response = r#"
        [
          {"id":"note-2","type":"supports","confidence":0.88,"reason":"Evidence overlaps"}
        ]
        "#;

        let parsed = parse_llm_rerank_response(response);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].target_id, "note-2");
        assert_eq!(parsed[0].link_type, "supports");
        assert_eq!(parsed[0].confidence, 0.88);
    }

    #[test]
    fn synthetic_bootstrap_handles_large_note_sets() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut service = LinkDiscoveryService::new(temp_dir.path().to_path_buf());
        let notes = (0..1000)
            .map(|index| {
                make_note(
                    &format!("note-{index}"),
                    &format!("Synthetic Note {index}"),
                    &format!(
                        "Synthetic body {index}. Machine learning topic {}.",
                        index % 25
                    ),
                    &[if index % 2 == 0 { "ai" } else { "research" }],
                    &[],
                )
            })
            .collect::<Vec<_>>();

        service.bootstrap(&notes);

        assert_eq!(service.profiles.len(), 1000);
        assert!(!service.queue.is_empty());
    }
}

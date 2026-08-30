use crate::models::canvas::{
    CanvasSession, CanvasViewport, Debate, LLMNodePositionUpdate, PromptTile, SessionCreate,
    SessionMeta, SessionUpdate, TilePosition, TilePositionUpdate,
};
use crate::services::atomic_io::write_atomic;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use walkdir::WalkDir;

pub type TileResponseUpdate = (
    String,
    String,
    crate::models::canvas::ResponseStatus,
    Option<String>,
    Option<f64>,
);

/// Service for managing canvas sessions (JSON file storage) with in-memory cache.
///
/// The cache eliminates repeated disk reads — every get_session/list_sessions call
/// returns from memory. Writes update the cache first then flush to disk (write-through).
#[derive(Clone)]
pub struct CanvasStore {
    data_path: PathBuf,
    /// Full session cache, populated lazily on first access per session.
    session_cache: HashMap<String, CanvasSession>,
    /// Whether the session list cache has been populated from disk.
    list_cache_ready: bool,
    pending_bases: HashMap<String, CanvasSession>,
    event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
    root_capability: Option<Arc<crate::services::twin_events::AnchoredRoot>>,
}

impl CanvasStore {
    pub fn new(data_path: PathBuf) -> Self {
        Self::with_event_recorder(
            data_path,
            Arc::new(crate::services::twin_events::NoopEventRecorder),
        )
    }

    pub fn with_event_recorder(
        data_path: PathBuf,
        event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
    ) -> Self {
        // Ensure directory exists
        std::fs::create_dir_all(&data_path).ok();
        let root_capability = crate::services::twin_events::AnchoredRoot::open(&data_path)
            .ok()
            .map(Arc::new);
        Self {
            data_path,
            session_cache: HashMap::new(),
            list_cache_ready: false,
            pending_bases: HashMap::new(),
            event_recorder,
            root_capability,
        }
    }

    fn collect_descendant_tile_ids(
        session: &CanvasSession,
        tile_id: &str,
        parent_model_id: Option<&str>,
    ) -> HashSet<String> {
        let mut descendants = HashSet::new();
        let mut frontier: Vec<String> = session
            .prompt_tiles
            .iter()
            .filter(|tile| {
                tile.parent_tile_id.as_deref() == Some(tile_id)
                    && parent_model_id.map_or(true, |model_id| {
                        tile.parent_model_id.as_deref() == Some(model_id)
                    })
            })
            .map(|tile| tile.id.clone())
            .collect();

        while let Some(current_id) = frontier.pop() {
            if !descendants.insert(current_id.clone()) {
                continue;
            }

            for child_id in session
                .prompt_tiles
                .iter()
                .filter(|tile| tile.parent_tile_id.as_deref() == Some(current_id.as_str()))
                .map(|tile| tile.id.clone())
            {
                frontier.push(child_id);
            }
        }

        descendants
    }

    fn debate_uses_removed_tiles(debate: &Debate, removed_tile_ids: &HashSet<String>) -> bool {
        !removed_tile_ids.is_empty()
            && debate
                .source_tile_ids
                .iter()
                .any(|source_tile_id| removed_tile_ids.contains(source_tile_id))
    }

    fn debate_uses_deleted_response(debate: &Debate, tile_id: &str, model_id: &str) -> bool {
        debate
            .source_tile_ids
            .iter()
            .any(|source_tile_id| source_tile_id == tile_id)
            && debate
                .participating_models
                .iter()
                .any(|participating_model| participating_model == model_id)
    }

    /// Ensure all sessions are loaded into cache (called once, on first list)
    fn ensure_list_cache(&mut self) {
        if self.list_cache_ready {
            return;
        }
        for entry in WalkDir::new(&self.data_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(session) = self.read_session_file(path) {
                    self.session_cache.insert(session.id.clone(), session);
                }
            }
        }
        self.list_cache_ready = true;
    }

    /// Drop every process-local Canvas snapshot so the next read is sourced
    /// from durable bytes captured after the caller's authority ticket.
    pub(crate) fn reload_authoritative_state(&mut self) {
        self.session_cache.clear();
        self.pending_bases.clear();
        self.list_cache_ready = false;
    }

    /// List all sessions (metadata only)
    pub fn list_sessions(&mut self) -> Result<Vec<SessionMeta>> {
        self.ensure_list_cache();
        let mut sessions: Vec<SessionMeta> =
            self.session_cache.values().map(SessionMeta::from).collect();

        // Sort by updated_at descending
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    /// Get a full session by ID (from cache, falls back to disk)
    pub fn get_session(&mut self, id: &str) -> Result<CanvasSession> {
        Self::validate_session_id(id)?;
        if let Some(session) = self.session_cache.get(id) {
            return Ok(session.clone());
        }
        // Cache miss: load from disk
        let path = self.session_path(id);
        let session = self
            .read_session_file(&path)
            .with_context(|| format!("Session not found: {}", id))?;
        self.session_cache.insert(id.to_string(), session.clone());
        Ok(session)
    }

    /// Get a mutable reference to a cached session, loading from disk if needed
    fn get_session_mut(&mut self, id: &str) -> Result<&mut CanvasSession> {
        if !self.session_cache.contains_key(id) {
            let path = self.session_path(id);
            let session = self
                .read_session_file(&path)
                .with_context(|| format!("Session not found: {}", id))?;
            self.session_cache.insert(id.to_string(), session);
        }
        if !self.pending_bases.contains_key(id) {
            let base = self
                .session_cache
                .get(id)
                .expect("session was loaded")
                .clone();
            self.pending_bases.insert(id.to_string(), base);
        }
        Ok(self.session_cache.get_mut(id).unwrap())
    }

    /// Create a new session
    pub fn create_session(&mut self, create: SessionCreate) -> Result<CanvasSession> {
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();

        let session = CanvasSession {
            id: id.clone(),
            title: create.title,
            description: create.description,
            prompt_tiles: Vec::new(),
            debates: Vec::new(),
            viewport: Default::default(),
            created_at: now,
            updated_at: now,
            tags: create.tags,
            status: "draft".to_string(),
            pinned_note_ids: Vec::new(),
        };

        self.write_session_file(&session)?;
        self.session_cache.insert(id, session.clone());
        Ok(session)
    }

    /// Update an existing session
    pub fn update_session(&mut self, id: &str, update: SessionUpdate) -> Result<CanvasSession> {
        let session = self.get_session_mut(id)?;

        if let Some(title) = update.title {
            session.title = title;
        }
        if let Some(description) = update.description {
            session.description = Some(description);
        }
        if let Some(tags) = update.tags {
            session.tags = tags;
        }
        if let Some(status) = update.status {
            session.status = status;
        }
        if let Some(viewport) = update.viewport {
            session.viewport = viewport;
        }
        if let Some(pinned_note_ids) = update.pinned_note_ids {
            session.pinned_note_ids = pinned_note_ids;
        }

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(session)
    }

    /// Delete a session
    pub fn delete_session(&mut self, id: &str) -> Result<()> {
        Self::validate_session_id(id)?;
        let path = self.session_path(id);
        if self.event_recorder.is_noop() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to delete session: {}", id))?;
        } else {
            self.event_recorder
                .commit_mutation(
                    crate::services::twin_events::MutationOrigin::Local,
                    crate::models::twin_event::CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("canvas")
                        .map_err(anyhow::Error::msg)?,
                    vec![crate::services::twin_events::TargetMutation::tombstone(
                        crate::services::twin_events::TargetKind::CanvasJson,
                        format!("{id}.json"),
                    )],
                    Vec::new(),
                )
                .map_err(anyhow::Error::new)?;
        }
        self.session_cache.remove(id);
        self.pending_bases.remove(id);
        Ok(())
    }

    /// Add a prompt tile to a session
    pub fn add_tile(&mut self, session_id: &str, tile: PromptTile) -> Result<CanvasSession> {
        self.add_tile_internal(session_id, tile, None)
            .map(|(session, _)| session)
    }

    pub(crate) fn add_tile_expecting_authority(
        &mut self,
        session_id: &str,
        tile: PromptTile,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(CanvasSession, crate::services::twin_events::MutationCommit)> {
        self.add_tile_internal(session_id, tile, Some(expected))
    }

    fn add_tile_internal(
        &mut self,
        session_id: &str,
        tile: PromptTile,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(CanvasSession, crate::services::twin_events::MutationCommit)> {
        let session = self.get_session_mut(session_id)?;
        session.prompt_tiles.push(tile);
        session.updated_at = Utc::now();
        let session = session.clone();
        let commit = self.write_session_file_internal(&session, None, expected)?;
        Ok((session, commit))
    }

    pub fn add_decision_tile(
        &mut self,
        session_id: &str,
        tile: PromptTile,
        twin_store: &mut crate::services::twin::TwinStore,
        decision: crate::models::twin::DecisionEpisodeCreate,
    ) -> Result<CanvasSession> {
        self.add_decision_tile_internal(session_id, tile, twin_store, decision, None)
            .map(|(session, _)| session)
    }

    pub(crate) fn add_decision_tile_expecting_authority(
        &mut self,
        session_id: &str,
        tile: PromptTile,
        twin_store: &mut crate::services::twin::TwinStore,
        decision: crate::models::twin::DecisionEpisodeCreate,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(CanvasSession, crate::services::twin_events::MutationCommit)> {
        self.add_decision_tile_internal(session_id, tile, twin_store, decision, Some(expected))
    }

    fn add_decision_tile_internal(
        &mut self,
        session_id: &str,
        tile: PromptTile,
        twin_store: &mut crate::services::twin::TwinStore,
        decision: crate::models::twin::DecisionEpisodeCreate,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(CanvasSession, crate::services::twin_events::MutationCommit)> {
        let session = self.get_session_mut(session_id)?;
        session.prompt_tiles.push(tile);
        session.updated_at = Utc::now();
        let session = session.clone();
        let commit = self.write_session_file_internal(
            &session,
            Some((twin_store, decision)),
            expected,
        )?;
        Ok((session, commit))
    }

    /// Delete a prompt tile and its children from a session
    pub fn delete_tile(&mut self, session_id: &str, tile_id: &str) -> Result<()> {
        let session = self.get_session_mut(session_id)?;
        let mut removed_tile_ids = Self::collect_descendant_tile_ids(session, tile_id, None);
        removed_tile_ids.insert(tile_id.to_string());

        // Remove tile and its full descendant tree.
        session
            .prompt_tiles
            .retain(|tile| !removed_tile_ids.contains(&tile.id));

        // Remove direct debate nodes and any debates tied to the deleted subtree.
        session
            .debates
            .retain(|debate| !Self::debate_uses_removed_tiles(debate, &removed_tile_ids));

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Delete a single model response from a tile
    pub fn delete_response(
        &mut self,
        session_id: &str,
        tile_id: &str,
        model_id: &str,
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;
        let descendants = Self::collect_descendant_tile_ids(session, tile_id, Some(model_id));

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            tile.responses.remove(model_id);
            tile.models.retain(|m| m != model_id);
        }

        session
            .prompt_tiles
            .retain(|tile| !descendants.contains(&tile.id));
        session.debates.retain(|debate| {
            !Self::debate_uses_removed_tiles(debate, &descendants)
                && !Self::debate_uses_deleted_response(debate, tile_id, model_id)
        });

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Update viewport zoom/pan state
    pub fn update_viewport(&mut self, session_id: &str, viewport: CanvasViewport) -> Result<()> {
        let session = self.get_session_mut(session_id)?;
        session.viewport = viewport;
        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Update a tile's position
    pub fn update_tile_position(
        &mut self,
        session_id: &str,
        tile_id: &str,
        position: TilePositionUpdate,
    ) -> Result<CanvasSession> {
        let session = self.get_session_mut(session_id)?;

        // Check prompt tiles
        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            tile.position.x = position.x;
            tile.position.y = position.y;
            if let Some(width) = position.width {
                tile.position.width = width;
            }
            if let Some(height) = position.height {
                tile.position.height = height;
            }
        }

        // Also check debates
        if let Some(debate) = session.debates.iter_mut().find(|d| d.id == tile_id) {
            debate.position.x = position.x;
            debate.position.y = position.y;
            if let Some(width) = position.width {
                debate.position.width = width;
            }
            if let Some(height) = position.height {
                debate.position.height = height;
            }
        }

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(session)
    }

    /// Update an individual LLM response node's position
    pub fn update_llm_node_position(
        &mut self,
        session_id: &str,
        tile_id: &str,
        model_id: &str,
        position: LLMNodePositionUpdate,
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            if let Some(response) = tile.responses.get_mut(model_id) {
                response.position.x = position.x;
                response.position.y = position.y;
                if let Some(width) = position.width {
                    response.position.width = width;
                }
                if let Some(height) = position.height {
                    response.position.height = height;
                }
            }
        }

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Batch update positions for auto-arrange
    pub fn batch_update_positions(
        &mut self,
        session_id: &str,
        positions: HashMap<String, TilePosition>,
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;

        for (node_id, position) in &positions {
            let parts: Vec<&str> = node_id.splitn(3, ':').collect();

            match parts.first().copied() {
                Some("prompt") if parts.len() >= 2 => {
                    let tile_id = parts[1];
                    if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
                        tile.position = position.clone();
                    }
                }
                Some("llm") if parts.len() >= 3 => {
                    let tile_id = parts[1];
                    let model_id = parts[2];
                    if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
                        if let Some(response) = tile.responses.get_mut(model_id) {
                            response.position = position.clone();
                        }
                    }
                }
                Some("debate") if parts.len() >= 2 => {
                    let debate_id = parts[1];
                    if let Some(debate) = session.debates.iter_mut().find(|d| d.id == debate_id) {
                        debate.position = position.clone();
                    }
                }
                _ => {}
            }
        }

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Add a debate to a session
    pub fn add_debate(&mut self, session_id: &str, debate: Debate) -> Result<()> {
        self.add_debate_internal(session_id, debate, None).map(|_| ())
    }

    pub(crate) fn add_debate_expecting_authority(
        &mut self,
        session_id: &str,
        debate: Debate,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.add_debate_internal(session_id, debate, Some(expected))
    }

    fn add_debate_internal(
        &mut self,
        session_id: &str,
        debate: Debate,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        let session = self.get_session_mut(session_id)?;
        session.debates.push(debate);
        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file_internal(&session, None, expected)
    }

    /// Update a debate's rounds
    pub fn update_debate(&mut self, session_id: &str, debate: &Debate) -> Result<()> {
        self.update_debate_internal(session_id, debate, None)
            .map(|_| ())
    }

    pub(crate) fn update_debate_expecting_authority(
        &mut self,
        session_id: &str,
        debate: &Debate,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.update_debate_internal(session_id, debate, Some(expected))
    }

    fn update_debate_internal(
        &mut self,
        session_id: &str,
        debate: &Debate,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        let session = self.get_session_mut(session_id)?;

        if let Some(existing) = session.debates.iter_mut().find(|d| d.id == debate.id) {
            *existing = debate.clone();
        }

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file_internal(&session, None, expected)
    }

    /// Batch update multiple tile responses in a single read/write cycle.
    /// Used after parallel streaming completes to avoid N separate file I/O operations.
    pub fn batch_update_tile_responses(
        &mut self,
        session_id: &str,
        tile_id: &str,
        updates: &[TileResponseUpdate],
    ) -> Result<()> {
        self.batch_update_tile_responses_internal(session_id, tile_id, updates, None)
            .map(|_| ())
    }

    pub(crate) fn batch_update_tile_responses_expecting_authority(
        &mut self,
        session_id: &str,
        tile_id: &str,
        updates: &[TileResponseUpdate],
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.batch_update_tile_responses_internal(session_id, tile_id, updates, Some(expected))
    }

    fn batch_update_tile_responses_internal(
        &mut self,
        session_id: &str,
        tile_id: &str,
        updates: &[TileResponseUpdate],
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        let session = self.get_session_mut(session_id)?;

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            for (model_id, content, status, error, cost_usd) in updates {
                if let Some(response) = tile.responses.get_mut(model_id) {
                    response.content = content.clone();
                    response.status = status.clone();
                    response.error = error.clone();
                    response.cost_usd = *cost_usd;
                }
            }
        }

        let session = session.clone();
        self.write_session_file_internal(&session, None, expected)
    }

    /// Update a tile's response content (for streaming)
    pub fn update_tile_response(
        &mut self,
        session_id: &str,
        tile_id: &str,
        model_id: &str,
        content: &str,
        status: crate::models::canvas::ResponseStatus,
        error: Option<&str>,
        cost_usd: Option<f64>,
    ) -> Result<()> {
        self.update_tile_response_internal(
            session_id,
            tile_id,
            model_id,
            content,
            status,
            error,
            cost_usd,
            None,
        )
        .map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_tile_response_expecting_authority(
        &mut self,
        session_id: &str,
        tile_id: &str,
        model_id: &str,
        content: &str,
        status: crate::models::canvas::ResponseStatus,
        error: Option<&str>,
        cost_usd: Option<f64>,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.update_tile_response_internal(
            session_id,
            tile_id,
            model_id,
            content,
            status,
            error,
            cost_usd,
            Some(expected),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn update_tile_response_internal(
        &mut self,
        session_id: &str,
        tile_id: &str,
        model_id: &str,
        content: &str,
        status: crate::models::canvas::ResponseStatus,
        error: Option<&str>,
        cost_usd: Option<f64>,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        let session = self.get_session_mut(session_id)?;

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            if let Some(response) = tile.responses.get_mut(model_id) {
                response.content = content.to_string();
                response.status = status;
                response.error = error.map(str::to_string);
                response.cost_usd = cost_usd;
            }
        }

        let session = session.clone();
        self.write_session_file_internal(&session, None, expected)
    }

    /// Save a full session object (used after streaming completes)
    pub fn save_session(&mut self, session: &CanvasSession) -> Result<()> {
        self.save_session_internal(session, None).map(|_| ())
    }

    pub(crate) fn save_session_expecting_authority(
        &mut self,
        session: &CanvasSession,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.save_session_internal(session, Some(expected))
    }

    fn save_session_internal(
        &mut self,
        session: &CanvasSession,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        if let Some(base) = self.session_cache.get(&session.id).cloned() {
            self.pending_bases.insert(session.id.clone(), base);
        }
        self.session_cache
            .insert(session.id.clone(), session.clone());
        self.write_session_file_internal(session, None, expected)
    }

    /// Validate that a session ID doesn't contain path traversal sequences
    fn validate_session_id(id: &str) -> Result<()> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid session ID: {}", id);
        }
        Ok(())
    }

    /// Get the file path for a session ID
    fn session_path(&self, id: &str) -> PathBuf {
        self.data_path.join(format!("{}.json", id))
    }

    /// Read and parse a session file
    fn read_session_file(&self, path: &std::path::Path) -> Result<CanvasSession> {
        self.read_session_file_optional(path)?
            .ok_or_else(|| anyhow::anyhow!("Failed to read file: {:?}", path))
    }

    fn read_session_file_optional(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<CanvasSession>> {
        const CANVAS_JSON_LIMIT: usize = 16 * 1024 * 1024;
        let relative = path
            .strip_prefix(&self.data_path)
            .map_err(|_| anyhow::anyhow!("Canvas read escaped the configured store root"))?
            .to_string_lossy()
            .replace('\\', "/");
        let root = self.root_capability.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Canvas store root capability could not be acquired")
        })?;
        let Some(bytes) = root
            .read_bounded(&relative, CANVAS_JSON_LIMIT)
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse session: {:?}", path))
            .map(Some)
    }

    /// Write a session to file
    fn write_session_file(&mut self, session: &CanvasSession) -> Result<()> {
        self.write_session_file_internal(session, None, None)
            .map(|_| ())
    }

    fn write_session_file_internal(
        &mut self,
        session: &CanvasSession,
        mut decision: Option<(
            &mut crate::services::twin::TwinStore,
            crate::models::twin::DecisionEpisodeCreate,
        )>,
        expected_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        let path = self.session_path(&session.id);
        let candidate = session.clone();
        let cached_base = self.pending_bases.remove(&session.id);
        let before_error_fallback = self.read_session_file_optional(&path)?;
        if self.event_recorder.is_noop() {
            if decision.is_some() {
                anyhow::bail!("compound decision capture requires a mutation coordinator");
            }
            let content = serde_json::to_string_pretty(&candidate)?;
            write_atomic(&path, content.as_bytes())
                .with_context(|| format!("Failed to write session: {:?}", path))?;
            return Ok(crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
            });
        }

        let recorder = self.event_recorder.clone();
        let session_id = candidate.id.clone();
        let path_for_plan = path.clone();
        let mut committed_session = None;
        let mut committed_decision_trace = None;
        let mut planner = || {
            let durable_before = self
                .read_session_file_optional(&path_for_plan)
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let after = match (&cached_base, &durable_before) {
                (Some(base), Some(durable)) => {
                    merge_canvas_session_change(base, &candidate, durable).map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?
                }
                _ => candidate.clone(),
            };
            let content = serde_json::to_string_pretty(&after).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            let events = recorder.recorded_events()?;
            let mut drafts = crate::services::twin_events::canvas_transition_drafts(
                durable_before.as_ref(),
                &after,
                &events,
                crate::services::twin_events::digest_bytes(content.as_bytes()),
            )
            .map_err(crate::services::twin_events::MutationError::Invalid)?;
            let mut targets = vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::CanvasJson,
                format!("{session_id}.json"),
                content,
            )];
            if let Some((twin_store, create)) = decision.as_mut() {
                let (_episode, trace, values, decision_drafts) = twin_store
                    .plan_decision_episode_mutation(create.clone())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                targets.extend(twin_store.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?);
                drafts.extend(decision_drafts);
                committed_decision_trace = Some(trace);
            }
            committed_session = Some(after);
            let mut plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("canvas")
                    .map_err(crate::services::twin_events::MutationError::Invalid)?,
                targets,
                drafts,
            );
            if let Some(expected) = expected_authority.clone() {
                plan = plan.expecting_authority(expected);
            }
            Ok(Some(plan))
        };
        let persist = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        );
        let commit = match persist {
            Ok(commit) => commit,
            Err(error) => {
            match self.read_session_file(&path) {
                Ok(durable) => {
                    self.session_cache.insert(session.id.clone(), durable);
                }
                Err(_) => {
                    if let Some(before) = before_error_fallback {
                        self.session_cache.insert(session.id.clone(), before);
                    } else {
                        self.session_cache.remove(&session.id);
                    }
                }
            }
            return Err(anyhow::Error::new(error));
            }
        };
        if let Some(committed) = committed_session {
            self.session_cache.insert(session.id.clone(), committed);
        }
        if let (Some((twin_store, _)), Some(trace)) = (decision.as_mut(), committed_decision_trace)
        {
            twin_store.cache_committed_trace(trace);
        }
        Ok(commit)
    }
}

fn merge_canvas_session_change(
    base: &CanvasSession,
    candidate: &CanvasSession,
    durable: &CanvasSession,
) -> Result<CanvasSession> {
    let base = serde_json::to_value(base)?;
    let candidate = serde_json::to_value(candidate)?;
    let mut durable = serde_json::to_value(durable)?;
    merge_canvas_value(&base, &candidate, &mut durable);
    Ok(serde_json::from_value(durable)?)
}

fn merge_canvas_value(
    base: &serde_json::Value,
    candidate: &serde_json::Value,
    durable: &mut serde_json::Value,
) {
    if base == candidate {
        return;
    }
    match (base, candidate, durable) {
        (
            serde_json::Value::Object(base),
            serde_json::Value::Object(candidate),
            serde_json::Value::Object(durable),
        ) => {
            for (key, candidate_value) in candidate {
                match (base.get(key), durable.get_mut(key)) {
                    (Some(base_value), Some(durable_value)) => {
                        merge_canvas_value(base_value, candidate_value, durable_value)
                    }
                    _ => {
                        durable.insert(key.clone(), candidate_value.clone());
                    }
                }
            }
        }
        (
            serde_json::Value::Array(base),
            serde_json::Value::Array(candidate),
            serde_json::Value::Array(durable),
        ) if canvas_array_identity_key(base, candidate, durable).is_some() => {
            let key = canvas_array_identity_key(base, candidate, durable).expect("checked key");
            let base_ids = base
                .iter()
                .filter_map(|value| canvas_identity(value, key))
                .collect::<HashSet<_>>();
            let candidate_ids = candidate
                .iter()
                .filter_map(|value| canvas_identity(value, key))
                .collect::<HashSet<_>>();
            durable.retain(|value| {
                canvas_identity(value, key)
                    .is_none_or(|id| !base_ids.contains(&id) || candidate_ids.contains(&id))
            });
            for candidate_value in candidate {
                let Some(id) = canvas_identity(candidate_value, key) else {
                    continue;
                };
                let base_value = base
                    .iter()
                    .find(|value| canvas_identity(value, key).as_deref() == Some(id.as_str()));
                let durable_value = durable
                    .iter_mut()
                    .find(|value| canvas_identity(value, key).as_deref() == Some(id.as_str()));
                match (base_value, durable_value) {
                    (Some(base_value), Some(durable_value)) => {
                        merge_canvas_value(base_value, candidate_value, durable_value)
                    }
                    (None, None) => durable.push(candidate_value.clone()),
                    _ => {}
                }
            }
        }
        (_, candidate, durable) => *durable = candidate.clone(),
    }
}

fn canvas_array_identity_key(
    base: &[serde_json::Value],
    candidate: &[serde_json::Value],
    durable: &[serde_json::Value],
) -> Option<&'static str> {
    ["id", "round_number", "model_id"].into_iter().find(|key| {
        base.iter()
            .chain(candidate)
            .chain(durable)
            .all(|value| canvas_identity(value, key).is_some())
            && !(base.is_empty() && candidate.is_empty() && durable.is_empty())
    })
}

fn canvas_identity(value: &serde_json::Value, key: &str) -> Option<String> {
    let value = value.as_object()?.get(key)?;
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::canvas::{
        Debate, DebateResponse, DebateRound, ModelResponse, PromptTile, ResponseStatus,
        SessionCreate,
    };
    use crate::services::atomic_io::assert_no_tmp_siblings;
    use tempfile::tempdir;

    #[test]
    fn session_writes_are_atomic_with_no_tmp_litter() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = CanvasStore::new(temp_dir.path().to_path_buf());

        let session = store
            .create_session(SessionCreate {
                title: "Atomic Session".to_string(),
                description: Some("adoption test".to_string()),
                tags: Vec::new(),
            })
            .expect("session should be created");

        let session_file = temp_dir.path().join(format!("{}.json", session.id));
        let persisted = std::fs::read_to_string(&session_file).expect("session file should exist");
        assert!(persisted.contains("Atomic Session"));
        assert_no_tmp_siblings(temp_dir.path());
    }

    #[test]
    fn authoritative_reload_discards_a_session_cached_before_a_peer_write() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
        let session = store
            .create_session(SessionCreate {
                title: "Before".to_string(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        assert_eq!(store.get_session(&session.id).unwrap().title, "Before");

        let mut peer = CanvasStore::new(temp_dir.path().to_path_buf());
        peer.update_session(
            &session.id,
            crate::models::canvas::SessionUpdate {
                title: Some("After".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        store.reload_authoritative_state();
        assert_eq!(store.get_session(&session.id).unwrap().title, "After");
    }

    fn build_response(model_id: &str) -> ModelResponse {
        ModelResponse {
            model_id: model_id.to_string(),
            model_name: model_id.to_string(),
            status: ResponseStatus::Completed,
            ..ModelResponse::default()
        }
    }

    fn build_tile(
        id: &str,
        parent_tile_id: Option<&str>,
        parent_model_id: Option<&str>,
        model_ids: &[&str],
    ) -> PromptTile {
        let mut tile = PromptTile {
            id: id.to_string(),
            parent_tile_id: parent_tile_id.map(str::to_string),
            parent_model_id: parent_model_id.map(str::to_string),
            ..PromptTile::default()
        };

        tile.models = model_ids
            .iter()
            .map(|model_id| model_id.to_string())
            .collect();
        for model_id in model_ids {
            tile.responses
                .insert((*model_id).to_string(), build_response(model_id));
        }

        tile
    }

    fn build_debate(id: &str, source_tile_ids: &[&str], participating_models: &[&str]) -> Debate {
        Debate {
            id: id.to_string(),
            source_tile_ids: source_tile_ids
                .iter()
                .map(|tile_id| tile_id.to_string())
                .collect(),
            participating_models: participating_models
                .iter()
                .map(|model_id| model_id.to_string())
                .collect(),
            ..Debate::default()
        }
    }

    #[test]
    fn delete_tile_removes_all_descendants() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
        let session = store
            .create_session(SessionCreate {
                title: "Tree".to_string(),
                description: None,
                tags: Vec::new(),
            })
            .expect("session should be created");

        let root = PromptTile {
            id: "root".to_string(),
            ..PromptTile::default()
        };
        let child = PromptTile {
            id: "child".to_string(),
            parent_tile_id: Some("root".to_string()),
            ..PromptTile::default()
        };
        let grandchild = PromptTile {
            id: "grandchild".to_string(),
            parent_tile_id: Some("child".to_string()),
            ..PromptTile::default()
        };
        let unrelated = PromptTile {
            id: "unrelated".to_string(),
            ..PromptTile::default()
        };

        store
            .add_tile(&session.id, root)
            .expect("root should be added");
        store
            .add_tile(&session.id, child)
            .expect("child should be added");
        store
            .add_tile(&session.id, grandchild)
            .expect("grandchild should be added");
        store
            .add_tile(&session.id, unrelated)
            .expect("unrelated should be added");

        store
            .delete_tile(&session.id, "root")
            .expect("delete should succeed");

        let remaining_ids: Vec<String> = store
            .get_session(&session.id)
            .expect("session should still load")
            .prompt_tiles
            .into_iter()
            .map(|tile| tile.id)
            .collect();

        assert_eq!(remaining_ids, vec!["unrelated".to_string()]);
    }

    #[test]
    fn delete_tile_removes_debates_for_removed_subtree() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
        let session = store
            .create_session(SessionCreate {
                title: "Debates".to_string(),
                description: None,
                tags: Vec::new(),
            })
            .expect("session should be created");

        store
            .add_tile(&session.id, build_tile("root", None, None, &["model-a"]))
            .expect("root should be added");
        store
            .add_tile(
                &session.id,
                build_tile("child", Some("root"), Some("model-a"), &["model-a"]),
            )
            .expect("child should be added");
        store
            .add_tile(&session.id, build_tile("sibling", None, None, &["model-b"]))
            .expect("sibling should be added");
        store
            .add_debate(
                &session.id,
                build_debate("debate-root", &["child"], &["model-a"]),
            )
            .expect("root debate should be added");
        store
            .add_debate(
                &session.id,
                build_debate("debate-sibling", &["sibling"], &["model-b"]),
            )
            .expect("sibling debate should be added");

        store
            .delete_tile(&session.id, "root")
            .expect("delete should succeed");

        let session = store
            .get_session(&session.id)
            .expect("session should still load");
        let remaining_tile_ids: Vec<String> = session
            .prompt_tiles
            .into_iter()
            .map(|tile| tile.id)
            .collect();
        let remaining_debate_ids: Vec<String> = session
            .debates
            .into_iter()
            .map(|debate| debate.id)
            .collect();

        assert_eq!(remaining_tile_ids, vec!["sibling".to_string()]);
        assert_eq!(remaining_debate_ids, vec!["debate-sibling".to_string()]);
    }

    #[test]
    fn delete_response_removes_only_the_deleted_model_branch() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
        let session = store
            .create_session(SessionCreate {
                title: "Responses".to_string(),
                description: None,
                tags: Vec::new(),
            })
            .expect("session should be created");

        store
            .add_tile(
                &session.id,
                build_tile("root", None, None, &["model-a", "model-b"]),
            )
            .expect("root should be added");
        store
            .add_tile(
                &session.id,
                build_tile("branch-a", Some("root"), Some("model-a"), &["model-a"]),
            )
            .expect("branch-a should be added");
        store
            .add_tile(
                &session.id,
                build_tile(
                    "branch-a-child",
                    Some("branch-a"),
                    Some("model-a"),
                    &["model-a"],
                ),
            )
            .expect("branch-a-child should be added");
        store
            .add_tile(
                &session.id,
                build_tile("branch-b", Some("root"), Some("model-b"), &["model-b"]),
            )
            .expect("branch-b should be added");
        store
            .add_debate(
                &session.id,
                build_debate("debate-a", &["root"], &["model-a"]),
            )
            .expect("debate-a should be added");
        store
            .add_debate(
                &session.id,
                build_debate("debate-branch-a", &["branch-a"], &["model-a"]),
            )
            .expect("debate-branch-a should be added");
        store
            .add_debate(
                &session.id,
                build_debate("debate-b", &["root"], &["model-b"]),
            )
            .expect("debate-b should be added");

        store
            .delete_response(&session.id, "root", "model-a")
            .expect("delete response should succeed");

        let session = store
            .get_session(&session.id)
            .expect("session should still load");
        let remaining_tile_ids: Vec<String> = session
            .prompt_tiles
            .iter()
            .map(|tile| tile.id.clone())
            .collect();
        let remaining_debate_ids: Vec<String> = session
            .debates
            .iter()
            .map(|debate| debate.id.clone())
            .collect();
        let root = session
            .prompt_tiles
            .iter()
            .find(|tile| tile.id == "root")
            .expect("root tile should remain");

        assert_eq!(
            remaining_tile_ids,
            vec!["root".to_string(), "branch-b".to_string()]
        );
        assert_eq!(root.models, vec!["model-b".to_string()]);
        assert!(!root.responses.contains_key("model-a"));
        assert_eq!(remaining_debate_ids, vec!["debate-b".to_string()]);
    }

    #[test]
    fn coordinated_canvas_captures_persisted_prompt_and_completion_but_not_structure() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator);
        let session = store
            .create_session(SessionCreate {
                title: "Captured".to_string(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        assert!(event_store.ordered_events().unwrap().is_empty());

        let mut tile = PromptTile {
            id: "tile-captured".to_string(),
            prompt: "What next?".to_string(),
            models: vec!["model-a".to_string()],
            ..PromptTile::default()
        };
        tile.responses.insert(
            "model-a".to_string(),
            ModelResponse {
                id: "response-captured".to_string(),
                model_id: "model-a".to_string(),
                model_name: "Model A".to_string(),
                status: ResponseStatus::Pending,
                ..ModelResponse::default()
            },
        );
        store.add_tile(&session.id, tile).unwrap();
        let events = event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].payload,
            crate::models::twin_event::TwinEventPayload::ConversationTurnRecorded(_)
        ));

        store
            .update_tile_response(
                &session.id,
                "tile-captured",
                "model-a",
                "A durable answer",
                ResponseStatus::Completed,
                None,
                Some(0.01),
            )
            .unwrap();
        let events = event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[1].payload,
            crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(_)
        ));

        store
            .update_viewport(
                &session.id,
                CanvasViewport {
                    x: 12.0,
                    y: 24.0,
                    zoom: 0.8,
                },
            )
            .unwrap();
        store
            .delete_response(&session.id, "tile-captured", "model-a")
            .unwrap();
        assert_eq!(event_store.ordered_events().unwrap().len(), 2);
    }

    #[test]
    fn decision_submission_recovers_canvas_episode_and_trace_as_one_event_group() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut canvas = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        let mut twin = crate::services::twin::TwinStore::with_event_recorder(
            data.join("twin").join("scope-one"),
            data.join("twin"),
            coordinator.clone(),
        );
        let session = canvas
            .create_session(SessionCreate {
                title: "Decision".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        let tile = PromptTile {
            id: "decision-tile".into(),
            prompt_type: crate::models::canvas::PromptType::Decision,
            prompt: "Ship now?".into(),
            decision_episode_id: Some("decision-episode".into()),
            decision_metadata: Some(crate::models::canvas::DecisionPromptMetadata {
                decision: "Ship now?".into(),
                options: vec!["Ship".into(), "Wait".into()],
                stakes: Some("Launch quality".into()),
                initial_leaning: Some("Ship".into()),
                review_date: None,
            }),
            ..PromptTile::default()
        };
        let create = crate::models::twin::DecisionEpisodeCreate {
            id: "decision-episode".into(),
            session_id: session.id.clone(),
            tile_id: tile.id.clone(),
            decision: "Ship now?".into(),
            options: vec!["Ship".into(), "Wait".into()],
            stakes: Some("Launch quality".into()),
            initial_leaning: Some("Ship".into()),
            review_date: None,
            primitive_assessment: Default::default(),
            context_version: Some("test-v1".into()),
        };

        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
        assert!(canvas
            .add_decision_tile(&session.id, tile, &mut twin, create)
            .is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);

        let captured = event_store.ordered_events().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(matches!(
            captured[0].payload,
            crate::models::twin_event::TwinEventPayload::ConversationTurnRecorded(_)
        ));
        assert!(matches!(
            captured[1].payload,
            crate::models::twin_event::TwinEventPayload::DecisionRecorded(_)
        ));
        assert_eq!(
            captured[1].causal_parents,
            vec![captured[0].event_id.clone()]
        );
        assert_eq!(
            twin.get_decision_episode("decision-episode")
                .unwrap()
                .tile_id,
            "decision-tile"
        );
        assert!(twin
            .get_session_trace(&session.id)
            .unwrap()
            .events
            .iter()
            .any(|event| event.event_type
                == crate::models::twin::TraceEventType::DecisionEpisodeCreated));
    }

    #[test]
    fn visible_decision_response_precedes_sealed_prediction_or_explicit_failure() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut canvas = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        let mut twin = crate::services::twin::TwinStore::with_event_recorder(
            data.join("twin/scope-one"),
            data.join("twin"),
            coordinator.clone(),
        );
        let session = canvas
            .create_session(SessionCreate {
                title: "Sequenced prediction".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();

        for (suffix, prediction_succeeds) in [("sealed", true), ("failed", false)] {
            let tile_id = format!("tile-{suffix}");
            let episode_id = format!("episode-{suffix}");
            let mut tile = PromptTile {
                id: tile_id.clone(),
                prompt_type: crate::models::canvas::PromptType::Decision,
                prompt: "Ship?".into(),
                models: vec!["model-a".into()],
                decision_episode_id: Some(episode_id.clone()),
                ..PromptTile::default()
            };
            tile.responses.insert(
                "model-a".into(),
                ModelResponse {
                    id: format!("response-{suffix}"),
                    model_id: "model-a".into(),
                    model_name: "Model A".into(),
                    status: ResponseStatus::Pending,
                    ..ModelResponse::default()
                },
            );
            let create = crate::models::twin::DecisionEpisodeCreate {
                id: episode_id.clone(),
                session_id: session.id.clone(),
                tile_id: tile_id.clone(),
                decision: "Ship?".into(),
                options: vec!["Ship".into(), "Wait".into()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: Default::default(),
                context_version: Some("test-v1".into()),
            };
            let expected = coordinator.current_authority_token().unwrap();
            let (_, prompt_commit) = canvas
                .add_decision_tile_expecting_authority(
                    &session.id,
                    tile,
                    &mut twin,
                    create,
                    expected,
                )
                .unwrap();
            let prompt_epoch = prompt_commit.authority_token.unwrap();
            let response_commit = canvas
                .batch_update_tile_responses_expecting_authority(
                    &session.id,
                    &tile_id,
                    &[(
                        "model-a".into(),
                        format!("visible-{suffix}"),
                        ResponseStatus::Completed,
                        None,
                        None,
                    )],
                    prompt_epoch,
                )
                .unwrap();
            let response_epoch = response_commit.authority_token.unwrap();

            if prediction_succeeds {
                let (_, commit) = twin
                    .attach_twin_prediction_expecting_authority(
                        &episode_id,
                        crate::models::twin::TwinPredictionDraft {
                            predicted_option: "Ship".into(),
                            matched_option_index: Some(0),
                            confidence: Some(0.8),
                            rationale: Some("evidence".into()),
                            parse_mode: "strict_json".into(),
                        },
                        "model-a",
                        "test-v1",
                        response_epoch,
                    )
                    .unwrap();
                assert!(commit.authority_token.is_some());
            } else {
                let commit = twin
                    .mark_twin_prediction_failed_expecting_authority(
                        &episode_id,
                        response_epoch,
                    )
                    .unwrap();
                assert!(commit.authority_token.is_some());
            }

            let durable_session = canvas.get_session(&session.id).unwrap();
            assert_eq!(
                durable_session
                    .prompt_tiles
                    .iter()
                    .find(|tile| tile.id == tile_id)
                    .unwrap()
                    .responses["model-a"]
                    .content,
                format!("visible-{suffix}")
            );
            let episode = twin.get_decision_episode(&episode_id).unwrap();
            if prediction_succeeds {
                assert!(episode.twin_prediction.is_some());
            } else {
                assert_eq!(episode.prediction_status.as_deref(), Some("failed"));
                assert!(episode.twin_prediction.is_none());
                let after_failure = coordinator.current_authority_token().unwrap();
                let duplicate = twin
                    .mark_twin_prediction_failed_expecting_authority(
                        &episode_id,
                        after_failure.clone(),
                    )
                    .unwrap();
                assert!(duplicate.authority_token.is_none());
                assert_eq!(coordinator.current_authority_token().unwrap(), after_failure);
            }
        }
    }

    #[test]
    fn coordinated_canvas_failure_restores_cache_and_persisted_session() {
        let root = tempdir().unwrap();
        let canvas = root.path().join("canvas");
        let session = CanvasStore::new(canvas.clone())
            .create_session(SessionCreate {
                title: "Stable".to_string(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        let mut store = CanvasStore::with_event_recorder(
            canvas.clone(),
            std::sync::Arc::new(crate::services::twin_events::UnavailableEventRecorder::new(
                "injected persistence failure",
            )),
        );
        store.get_session(&session.id).unwrap();
        let before = std::fs::read(canvas.join(format!("{}.json", session.id))).unwrap();
        let result = store.add_tile(
            &session.id,
            PromptTile {
                id: "must-not-stick".to_string(),
                prompt: "Do not persist".to_string(),
                ..PromptTile::default()
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(canvas.join(format!("{}.json", session.id))).unwrap(),
            before
        );
        assert!(store
            .get_session(&session.id)
            .unwrap()
            .prompt_tiles
            .is_empty());
    }

    #[test]
    fn interrupted_canvas_target_keeps_cache_equal_to_durable_bytes_until_recovery() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        let session = store
            .create_session(SessionCreate {
                title: "Interrupted".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
        assert!(store
            .add_tile(
                &session.id,
                PromptTile {
                    id: "durable-before-event".into(),
                    prompt: "Persist me once".into(),
                    ..PromptTile::default()
                },
            )
            .is_err());

        let path = data.join("canvas").join(format!("{}.json", session.id));
        let durable: CanvasSession = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let cached = store.get_session(&session.id).unwrap();
        assert_eq!(cached.prompt_tiles.len(), durable.prompt_tiles.len());
        assert_eq!(cached.prompt_tiles[0].id, durable.prompt_tiles[0].id);
        assert!(event_store.ordered_events().unwrap().is_empty());
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 1);
        assert_eq!(event_store.ordered_events().unwrap().len(), 1);
    }

    #[test]
    fn target_only_canvas_write_uses_journal_and_recovers_without_event() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        let session = store
            .create_session(SessionCreate {
                title: "Layout journal".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();

        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
        assert!(store
            .update_viewport(
                &session.id,
                CanvasViewport {
                    x: 7.0,
                    y: 9.0,
                    zoom: 0.75,
                },
            )
            .is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 1);
        assert!(event_store.ordered_events().unwrap().is_empty());
        let durable = store.get_session(&session.id).unwrap();
        assert_eq!(durable.viewport.x, 7.0);
        assert_eq!(durable.viewport.y, 9.0);
    }

    #[test]
    fn pending_canvas_change_is_recovered_before_fresh_session_plan() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut first = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        let session = first
            .create_session(SessionCreate {
                title: "Fresh plan".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        let mut stale = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        stale.get_session(&session.id).unwrap();

        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
        assert!(first
            .add_tile(
                &session.id,
                PromptTile {
                    id: "recovered-tile".into(),
                    prompt: "first".into(),
                    ..PromptTile::default()
                },
            )
            .is_err());
        stale
            .add_tile(
                &session.id,
                PromptTile {
                    id: "fresh-tile".into(),
                    prompt: "second".into(),
                    ..PromptTile::default()
                },
            )
            .unwrap();
        let durable = stale.get_session(&session.id).unwrap();
        let ids = durable
            .prompt_tiles
            .iter()
            .map(|tile| tile.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["recovered-tile", "fresh-tile"]);
        assert_eq!(event_store.ordered_events().unwrap().len(), 2);
    }

    #[test]
    fn immediate_regeneration_supersedes_event_recovered_under_same_lock() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
        let session = store
            .create_session(SessionCreate {
                title: "Regenerate after crash".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        let mut tile = PromptTile {
            id: "tile".into(),
            prompt: "prompt".into(),
            models: vec!["model".into()],
            ..PromptTile::default()
        };
        tile.responses.insert(
            "model".into(),
            ModelResponse {
                id: "stable-response".into(),
                model_id: "model".into(),
                model_name: "Model".into(),
                status: ResponseStatus::Pending,
                ..ModelResponse::default()
            },
        );
        store.add_tile(&session.id, tile).unwrap();
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
        assert!(store
            .update_tile_response(
                &session.id,
                "tile",
                "model",
                "first completion",
                ResponseStatus::Completed,
                None,
                None,
            )
            .is_err());
        store
            .update_tile_response(
                &session.id,
                "tile",
                "model",
                "regenerated completion",
                ResponseStatus::Completed,
                None,
                None,
            )
            .unwrap();

        let response_events = event_store
            .ordered_events()
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event.payload,
                    crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(_)
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(response_events.len(), 2);
        assert_eq!(
            response_events[1].supersedes,
            vec![response_events[0].event_id.clone()]
        );
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }

    #[test]
    fn coordinated_canvas_batches_regeneration_and_debate_are_persisted_once_in_order() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let event_store =
            std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator);
        let session = store
            .create_session(SessionCreate {
                title: "Batch and debate".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        let mut tile = PromptTile {
            id: "tile-batch".into(),
            prompt: "Compare both answers".into(),
            models: vec!["model-z".into(), "model-a".into()],
            ..PromptTile::default()
        };
        for (model_id, response_id) in [("model-z", "response-z"), ("model-a", "response-a")] {
            tile.responses.insert(
                model_id.into(),
                ModelResponse {
                    id: response_id.into(),
                    model_id: model_id.into(),
                    model_name: model_id.into(),
                    status: ResponseStatus::Pending,
                    ..ModelResponse::default()
                },
            );
        }
        store.add_tile(&session.id, tile).unwrap();
        store
            .batch_update_tile_responses(
                &session.id,
                "tile-batch",
                &[
                    (
                        "model-z".into(),
                        "z answer".into(),
                        ResponseStatus::Completed,
                        None,
                        Some(0.02),
                    ),
                    (
                        "model-a".into(),
                        "a answer".into(),
                        ResponseStatus::Completed,
                        None,
                        Some(0.01),
                    ),
                ],
            )
            .unwrap();
        let events = event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 3);
        let response_ids = events[1..]
            .iter()
            .map(|event| match &event.payload {
                crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(payload) => {
                    payload.response_id.as_str()
                }
                other => panic!("unexpected batch payload: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(response_ids, vec!["response-a", "response-z"]);

        store
            .update_tile_response(
                &session.id,
                "tile-batch",
                "model-a",
                "partial",
                ResponseStatus::Streaming,
                None,
                None,
            )
            .unwrap();
        assert_eq!(event_store.ordered_events().unwrap().len(), 3);
        let prior_a = events[1].event_id.clone();
        store
            .update_tile_response(
                &session.id,
                "tile-batch",
                "model-a",
                "regenerated answer",
                ResponseStatus::Completed,
                None,
                Some(0.03),
            )
            .unwrap();
        let events = event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[3].supersedes, vec![prior_a]);

        let mut debate = Debate {
            id: "debate-stable".into(),
            participating_models: vec!["model-z".into(), "model-a".into()],
            ..Debate::default()
        };
        store.add_debate(&session.id, debate.clone()).unwrap();
        assert_eq!(event_store.ordered_events().unwrap().len(), 4);
        debate.rounds.push(DebateRound {
            round_number: 1,
            topic: "Which answer is stronger?".into(),
            responses: vec![
                DebateResponse {
                    model_id: "model-z".into(),
                    model_name: "Model Z".into(),
                    content: "Z case".into(),
                    stance: None,
                    cost_usd: Some(0.02),
                    provider: Some("openrouter".into()),
                    provenance: Some("canvas_debate_openrouter".into()),
                },
                DebateResponse {
                    model_id: "model-a".into(),
                    model_name: "Model A".into(),
                    content: "A case".into(),
                    stance: None,
                    cost_usd: Some(0.01),
                    provider: Some("openrouter".into()),
                    provenance: Some("canvas_debate_openrouter".into()),
                },
            ],
            created_at: Utc::now(),
        });
        store.update_debate(&session.id, &debate).unwrap();
        let events = event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 7);
        assert!(matches!(
            events[4].payload,
            crate::models::twin_event::TwinEventPayload::ConversationTurnRecorded(_)
        ));
        let debate_rows = events[5..]
            .iter()
            .map(|event| match &event.payload {
                crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(payload) => {
                    (payload.response_id.as_str(), payload.model_id.as_str())
                }
                other => panic!("unexpected debate payload: {other:?}"),
            })
            .collect::<Vec<_>>();
        let mut sorted_ids = debate_rows.iter().map(|row| row.0).collect::<Vec<_>>();
        sorted_ids.sort_unstable();
        assert_eq!(
            debate_rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            sorted_ids
        );
        let mut debate_models = debate_rows.iter().map(|row| row.1).collect::<Vec<_>>();
        debate_models.sort_unstable();
        assert_eq!(debate_models, vec!["model-a", "model-z"]);
    }
}

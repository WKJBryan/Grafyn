use crate::models::canvas::{
    CanvasSession, CanvasViewport, CanvasWorkingMemory, Debate, LLMNodePositionUpdate, PromptTile,
    SessionCreate, SessionMeta, SessionUpdate, TilePosition, TilePositionUpdate,
};
use crate::services::atomic_io::write_atomic;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

pub type TileResponseUpdate = (
    String,
    String,
    crate::models::canvas::ResponseStatus,
    Option<String>,
    Option<f64>,
);

pub(crate) fn scoped_canvas_path(
    data_path: impl AsRef<Path>,
    root_scope: &crate::models::twin_event::ContentDigest,
) -> PathBuf {
    data_path
        .as_ref()
        .join("canvas")
        .join("v1")
        .join(root_scope.as_str())
}

pub(crate) fn require_canvas_only_commit(
    commit: &crate::services::twin_events::MutationCommit,
) -> Result<()> {
    anyhow::ensure!(
        commit.authority_token.is_none(),
        "Canvas-only mutation unexpectedly advanced content authority"
    );
    Ok(())
}

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

    pub fn replace_root_path(&mut self, data_path: PathBuf) -> Result<()> {
        std::fs::create_dir_all(&data_path)
            .with_context(|| format!("Failed to create Canvas root: {}", data_path.display()))?;
        crate::services::twin_events::validate_real_directory(&data_path, "Canvas root")
            .map_err(anyhow::Error::new)?;
        let root_capability = crate::services::twin_events::AnchoredRoot::open(&data_path)
            .map_err(anyhow::Error::new)?;
        self.data_path = data_path;
        self.root_capability = Some(Arc::new(root_capability));
        self.reload_authoritative_state();
        Ok(())
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
            working_memory: CanvasWorkingMemory::default(),
            branch_memories: HashMap::new(),
        };

        self.write_session_file(&session)?;
        self.session_cache.insert(id, session.clone());
        Ok(session)
    }

    pub fn update_branch_memory(
        &mut self,
        session_id: &str,
        branch_key: &str,
        memory: CanvasWorkingMemory,
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;
        session
            .branch_memories
            .insert(branch_key.to_string(), memory);
        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    pub fn update_working_memory(
        &mut self,
        session_id: &str,
        memory: CanvasWorkingMemory,
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;
        session.working_memory = memory;
        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
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
            let commit = self
                .event_recorder
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
            require_canvas_only_commit(&commit)?;
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
        Self::validate_tile_twin_evidence(&tile)?;
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
        Self::validate_tile_twin_evidence(&tile)?;
        let session = self.get_session_mut(session_id)?;
        session.prompt_tiles.push(tile);
        session.updated_at = Utc::now();
        let session = session.clone();
        let commit =
            self.write_session_file_internal(&session, Some((twin_store, decision)), expected)?;
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
        self.add_debate_internal(session_id, debate, None)
            .map(|_| ())
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
            session_id, tile_id, model_id, content, status, error, cost_usd, None,
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
        Self::validate_session_twin_evidence(session)?;
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

    fn validate_tile_twin_evidence(tile: &PromptTile) -> Result<()> {
        tile.validate_twin_relationship_context(None)
            .map_err(anyhow::Error::msg)
    }

    fn validate_session_twin_evidence(session: &CanvasSession) -> Result<()> {
        for tile in &session.prompt_tiles {
            Self::validate_tile_twin_evidence(tile)?;
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

    fn read_session_file_optional(&self, path: &std::path::Path) -> Result<Option<CanvasSession>> {
        const CANVAS_JSON_LIMIT: usize = 16 * 1024 * 1024;
        let relative = path
            .strip_prefix(&self.data_path)
            .map_err(|_| anyhow::anyhow!("Canvas read escaped the configured store root"))?
            .to_string_lossy()
            .replace('\\', "/");
        let root = self
            .root_capability
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Canvas store root capability could not be acquired"))?;
        let Some(bytes) = root
            .read_bounded(&relative, CANVAS_JSON_LIMIT)
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        let session = serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse session: {:?}", path))?;
        Self::validate_session_twin_evidence(&session)
            .with_context(|| format!("Invalid Twin evidence in session: {:?}", path))?;
        Ok(Some(session))
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
        Self::validate_session_twin_evidence(session)?;
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
                postcommit_warning: false,
            });
        }

        let recorder = self.event_recorder.clone();
        let session_id = candidate.id.clone();
        let path_for_plan = path.clone();
        let mut committed_session = None;
        let mut committed_decision_trace = None;
        let mut planner = || {
            let durable_before =
                self.read_session_file_optional(&path_for_plan)
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
            Self::validate_session_twin_evidence(&after).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
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
#[path = "canvas_store_tests.rs"]
mod tests;

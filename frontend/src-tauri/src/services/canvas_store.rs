use crate::models::canvas::{
    CanvasSession, CanvasViewport, Debate, LLMNodePositionUpdate,
    PromptTile, SessionCreate, SessionMeta, SessionUpdate, TilePosition, TilePositionUpdate,
};
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use walkdir::WalkDir;

/// Service for managing canvas sessions (JSON file storage) with in-memory cache.
///
/// The cache eliminates repeated disk reads — every get_session/list_sessions call
/// returns from memory. Writes update the cache first then flush to disk (write-through).
#[derive(Debug, Clone)]
pub struct CanvasStore {
    data_path: PathBuf,
    /// Full session cache, populated lazily on first access per session.
    session_cache: HashMap<String, CanvasSession>,
    /// Whether the session list cache has been populated from disk.
    list_cache_ready: bool,
}

impl CanvasStore {
    pub fn new(data_path: PathBuf) -> Self {
        // Ensure directory exists
        std::fs::create_dir_all(&data_path).ok();
        Self {
            data_path,
            session_cache: HashMap::new(),
            list_cache_ready: false,
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

    /// List all sessions (metadata only)
    pub fn list_sessions(&mut self) -> Result<Vec<SessionMeta>> {
        self.ensure_list_cache();
        let mut sessions: Vec<SessionMeta> = self
            .session_cache
            .values()
            .map(SessionMeta::from)
            .collect();

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
        let session = self.read_session_file(&path)
            .with_context(|| format!("Session not found: {}", id))?;
        self.session_cache.insert(id.to_string(), session.clone());
        Ok(session)
    }

    /// Get a mutable reference to a cached session, loading from disk if needed
    fn get_session_mut(&mut self, id: &str) -> Result<&mut CanvasSession> {
        if !self.session_cache.contains_key(id) {
            let path = self.session_path(id);
            let session = self.read_session_file(&path)
                .with_context(|| format!("Session not found: {}", id))?;
            self.session_cache.insert(id.to_string(), session);
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
        std::fs::remove_file(&path).with_context(|| format!("Failed to delete session: {}", id))?;
        self.session_cache.remove(id);
        Ok(())
    }

    /// Add a prompt tile to a session
    pub fn add_tile(&mut self, session_id: &str, tile: PromptTile) -> Result<CanvasSession> {
        let session = self.get_session_mut(session_id)?;
        session.prompt_tiles.push(tile);
        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(session)
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
    pub fn delete_response(&mut self, session_id: &str, tile_id: &str, model_id: &str) -> Result<()> {
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
        let session = self.get_session_mut(session_id)?;
        session.debates.push(debate);
        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Update a debate's rounds
    pub fn update_debate(&mut self, session_id: &str, debate: &Debate) -> Result<()> {
        let session = self.get_session_mut(session_id)?;

        if let Some(existing) = session.debates.iter_mut().find(|d| d.id == debate.id) {
            *existing = debate.clone();
        }

        session.updated_at = Utc::now();
        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Batch update multiple tile responses in a single read/write cycle.
    /// Used after parallel streaming completes to avoid N separate file I/O operations.
    pub fn batch_update_tile_responses(
        &mut self,
        session_id: &str,
        tile_id: &str,
        updates: &[(
            String,
            String,
            crate::models::canvas::ResponseStatus,
            Option<String>,
        )],
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            for (model_id, content, status, error) in updates {
                if let Some(response) = tile.responses.get_mut(model_id) {
                    response.content = content.clone();
                    response.status = status.clone();
                    response.error = error.clone();
                }
            }
        }

        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
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
    ) -> Result<()> {
        let session = self.get_session_mut(session_id)?;

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            if let Some(response) = tile.responses.get_mut(model_id) {
                response.content = content.to_string();
                response.status = status;
                response.error = error.map(str::to_string);
            }
        }

        let session = session.clone();
        self.write_session_file(&session)?;
        Ok(())
    }

    /// Save a full session object (used after streaming completes)
    pub fn save_session(&mut self, session: &CanvasSession) -> Result<()> {
        self.session_cache.insert(session.id.clone(), session.clone());
        self.write_session_file(session)
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
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {:?}", path))?;

        serde_json::from_str(&content).with_context(|| format!("Failed to parse session: {:?}", path))
    }

    /// Write a session to file
    fn write_session_file(&self, session: &CanvasSession) -> Result<()> {
        let path = self.session_path(&session.id);
        let content = serde_json::to_string_pretty(session)?;

        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write session: {:?}", path))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::canvas::{Debate, ModelResponse, PromptTile, ResponseStatus, SessionCreate};
    use tempfile::tempdir;

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

        tile.models = model_ids.iter().map(|model_id| model_id.to_string()).collect();
        for model_id in model_ids {
            tile.responses
                .insert((*model_id).to_string(), build_response(model_id));
        }

        tile
    }

    fn build_debate(id: &str, source_tile_ids: &[&str], participating_models: &[&str]) -> Debate {
        Debate {
            id: id.to_string(),
            source_tile_ids: source_tile_ids.iter().map(|tile_id| tile_id.to_string()).collect(),
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

        store.add_tile(&session.id, root).expect("root should be added");
        store.add_tile(&session.id, child).expect("child should be added");
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
            .add_tile(&session.id, build_tile("child", Some("root"), Some("model-a"), &["model-a"]))
            .expect("child should be added");
        store
            .add_tile(&session.id, build_tile("sibling", None, None, &["model-b"]))
            .expect("sibling should be added");
        store
            .add_debate(&session.id, build_debate("debate-root", &["child"], &["model-a"]))
            .expect("root debate should be added");
        store
            .add_debate(&session.id, build_debate("debate-sibling", &["sibling"], &["model-b"]))
            .expect("sibling debate should be added");

        store
            .delete_tile(&session.id, "root")
            .expect("delete should succeed");

        let session = store.get_session(&session.id).expect("session should still load");
        let remaining_tile_ids: Vec<String> =
            session.prompt_tiles.into_iter().map(|tile| tile.id).collect();
        let remaining_debate_ids: Vec<String> =
            session.debates.into_iter().map(|debate| debate.id).collect();

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
            .add_tile(&session.id, build_tile("root", None, None, &["model-a", "model-b"]))
            .expect("root should be added");
        store
            .add_tile(&session.id, build_tile("branch-a", Some("root"), Some("model-a"), &["model-a"]))
            .expect("branch-a should be added");
        store
            .add_tile(
                &session.id,
                build_tile("branch-a-child", Some("branch-a"), Some("model-a"), &["model-a"]),
            )
            .expect("branch-a-child should be added");
        store
            .add_tile(&session.id, build_tile("branch-b", Some("root"), Some("model-b"), &["model-b"]))
            .expect("branch-b should be added");
        store
            .add_debate(&session.id, build_debate("debate-a", &["root"], &["model-a"]))
            .expect("debate-a should be added");
        store
            .add_debate(&session.id, build_debate("debate-branch-a", &["branch-a"], &["model-a"]))
            .expect("debate-branch-a should be added");
        store
            .add_debate(&session.id, build_debate("debate-b", &["root"], &["model-b"]))
            .expect("debate-b should be added");

        store
            .delete_response(&session.id, "root", "model-a")
            .expect("delete response should succeed");

        let session = store.get_session(&session.id).expect("session should still load");
        let remaining_tile_ids: Vec<String> =
            session.prompt_tiles.iter().map(|tile| tile.id.clone()).collect();
        let remaining_debate_ids: Vec<String> =
            session.debates.iter().map(|debate| debate.id.clone()).collect();
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
}

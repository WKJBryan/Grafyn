use crate::models::canvas::{
    CanvasSession, PromptTile, SessionCreate, SessionMeta, SessionUpdate, TilePosition,
    TilePositionUpdate,
};
use anyhow::{Context, Result};
use chrono::Utc;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Service for managing canvas sessions (JSON file storage)
#[derive(Debug, Clone)]
pub struct CanvasStore {
    data_path: PathBuf,
}

impl CanvasStore {
    pub fn new(data_path: PathBuf) -> Self {
        // Ensure directory exists
        std::fs::create_dir_all(&data_path).ok();
        Self { data_path }
    }

    /// List all sessions (metadata only)
    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let mut sessions = Vec::new();

        for entry in WalkDir::new(&self.data_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(session) = self.read_session_file(path) {
                    sessions.push(SessionMeta::from(&session));
                }
            }
        }

        // Sort by updated_at descending
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    /// Get a full session by ID
    pub fn get_session(&self, id: &str) -> Result<CanvasSession> {
        let path = self.session_path(id);
        self.read_session_file(&path)
            .with_context(|| format!("Session not found: {}", id))
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
        };

        self.write_session_file(&session)?;
        Ok(session)
    }

    /// Update an existing session
    pub fn update_session(&mut self, id: &str, update: SessionUpdate) -> Result<CanvasSession> {
        let mut session = self.get_session(id)?;

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

        session.updated_at = Utc::now();
        self.write_session_file(&session)?;
        Ok(session)
    }

    /// Delete a session
    pub fn delete_session(&mut self, id: &str) -> Result<()> {
        let path = self.session_path(id);
        std::fs::remove_file(&path).with_context(|| format!("Failed to delete session: {}", id))?;
        Ok(())
    }

    /// Add a prompt tile to a session
    pub fn add_tile(&mut self, session_id: &str, tile: PromptTile) -> Result<CanvasSession> {
        let mut session = self.get_session(session_id)?;
        session.prompt_tiles.push(tile);
        session.updated_at = Utc::now();
        self.write_session_file(&session)?;
        Ok(session)
    }

    /// Update a tile's position
    pub fn update_tile_position(
        &mut self,
        session_id: &str,
        tile_id: &str,
        position: TilePositionUpdate,
    ) -> Result<CanvasSession> {
        let mut session = self.get_session(session_id)?;

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

        session.updated_at = Utc::now();
        self.write_session_file(&session)?;
        Ok(session)
    }

    /// Update a tile's response content (for streaming)
    pub fn update_tile_response(
        &mut self,
        session_id: &str,
        tile_id: &str,
        model_id: &str,
        content: &str,
        status: crate::models::canvas::ResponseStatus,
    ) -> Result<()> {
        let mut session = self.get_session(session_id)?;

        if let Some(tile) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            if let Some(response) = tile.responses.get_mut(model_id) {
                response.content = content.to_string();
                response.status = status;
            }
        }

        self.write_session_file(&session)?;
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

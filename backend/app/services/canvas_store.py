"""Canvas session storage and persistence"""
import json
import logging
from pathlib import Path
from typing import List, Optional, Dict
from datetime import datetime, timezone
import uuid

from backend.app.models.canvas import (
    CanvasSession,
    CanvasSessionListItem,
    CanvasCreate,
    CanvasUpdate,
    PromptTile,
    DebateRound,
    TilePosition,
    TilePositionUpdate,
    ModelResponse,
    CanvasViewport,
    DebateMode,
)
from backend.app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()


class CanvasSessionStore:
    """Service for managing canvas session persistence"""

    def __init__(self, data_path: Optional[str] = None):
        self.data_path = Path(data_path or settings.canvas_data_path)
        self.data_path.mkdir(parents=True, exist_ok=True)
        self._sessions: Dict[str, CanvasSession] = {}
        self._load_all()

    def _session_path(self, session_id: str) -> Path:
        """Get file path for a session"""
        return self.data_path / f"{session_id}.json"

    def _load_all(self):
        """Load all sessions from disk"""
        for json_file in self.data_path.glob("*.json"):
            try:
                with open(json_file, "r", encoding="utf-8") as f:
                    data = json.load(f)
                    session = CanvasSession(**data)
                    self._sessions[session.id] = session
            except Exception as e:
                logger.error(f"Failed to load canvas session {json_file}: {e}")

        logger.info(f"Loaded {len(self._sessions)} canvas sessions")

    def _save(self, session: CanvasSession):
        """Save a session to disk"""
        try:
            with open(self._session_path(session.id), "w", encoding="utf-8") as f:
                json.dump(session.model_dump(mode="json"), f, indent=2, default=str)
        except Exception as e:
            logger.error(f"Failed to save canvas session {session.id}: {e}")

    def list_sessions(self) -> List[CanvasSessionListItem]:
        """List all canvas sessions as summary items"""
        items = []
        for session in self._sessions.values():
            items.append(
                CanvasSessionListItem(
                    id=session.id,
                    title=session.title,
                    description=session.description,
                    tile_count=len(session.prompt_tiles),
                    debate_count=len(session.debates),
                    created_at=session.created_at,
                    updated_at=session.updated_at,
                    tags=session.tags,
                    status=session.status,
                )
            )
        # Sort by updated_at descending
        items.sort(key=lambda x: x.updated_at, reverse=True)
        return items

    def get_session(self, session_id: str) -> Optional[CanvasSession]:
        """Get a specific session"""
        return self._sessions.get(session_id)

    def create_session(self, data: CanvasCreate) -> CanvasSession:
        """Create a new canvas session"""
        session_id = str(uuid.uuid4())
        now = datetime.now(timezone.utc)

        session = CanvasSession(
            id=session_id,
            title=data.title,
            description=data.description,
            tags=data.tags,
            created_at=now,
            updated_at=now,
        )

        self._sessions[session_id] = session
        self._save(session)
        logger.info(f"Created canvas session: {session_id}")
        return session

    def update_session(
        self, session_id: str, data: CanvasUpdate
    ) -> Optional[CanvasSession]:
        """Update session metadata"""
        session = self._sessions.get(session_id)
        if not session:
            return None

        update_data = data.model_dump(exclude_unset=True)
        for key, value in update_data.items():
            if value is not None:
                setattr(session, key, value)

        session.updated_at = datetime.now(timezone.utc)
        self._save(session)
        return session

    def delete_session(self, session_id: str) -> bool:
        """Delete a canvas session"""
        if session_id not in self._sessions:
            return False

        del self._sessions[session_id]
        path = self._session_path(session_id)
        if path.exists():
            path.unlink()

        logger.info(f"Deleted canvas session: {session_id}")
        return True

    def add_prompt_tile(
        self,
        session_id: str,
        prompt: str,
        models: List[str],
        system_prompt: Optional[str] = None,
        position: Optional[TilePosition] = None,
    ) -> Optional[PromptTile]:
        """Add a new prompt tile to a session"""
        session = self._sessions.get(session_id)
        if not session:
            return None

        tile_id = str(uuid.uuid4())

        # Calculate position if not provided (stagger tiles)
        if position is None:
            base_x = 50 + (len(session.prompt_tiles) % 3) * 450
            base_y = 50 + (len(session.prompt_tiles) // 3) * 400
            position = TilePosition(x=base_x, y=base_y)

        # Create response placeholders for each model
        responses = {}
        for model_id in models:
            response_id = str(uuid.uuid4())
            model_name = model_id.split("/")[-1] if "/" in model_id else model_id
            responses[model_id] = ModelResponse(
                id=response_id,
                model_id=model_id,
                model_name=model_name,
                status="pending",
            )

        tile = PromptTile(
            id=tile_id,
            prompt=prompt,
            system_prompt=system_prompt,
            models=models,
            responses=responses,
            position=position,
        )

        session.prompt_tiles.append(tile)
        session.updated_at = datetime.now(timezone.utc)
        self._save(session)

        logger.info(f"Added prompt tile {tile_id} to session {session_id}")
        return tile

    def update_tile_position(
        self, session_id: str, tile_id: str, position: TilePositionUpdate
    ) -> bool:
        """Update a tile's position on the canvas"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        # Check prompt tiles
        for tile in session.prompt_tiles:
            if tile.id == tile_id:
                tile.position.x = position.x
                tile.position.y = position.y
                if position.width is not None:
                    tile.position.width = position.width
                if position.height is not None:
                    tile.position.height = position.height
                session.updated_at = datetime.now(timezone.utc)
                self._save(session)
                return True

        # Check debate tiles
        for debate in session.debates:
            if debate.id == tile_id:
                debate.position.x = position.x
                debate.position.y = position.y
                if position.width is not None:
                    debate.position.width = position.width
                if position.height is not None:
                    debate.position.height = position.height
                session.updated_at = datetime.now(timezone.utc)
                self._save(session)
                return True

        return False

    def update_response_content(
        self,
        session_id: str,
        tile_id: str,
        model_id: str,
        content: str,
        status: str = "streaming",
    ) -> bool:
        """Update a model response's content (for streaming)"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        for tile in session.prompt_tiles:
            if tile.id == tile_id and model_id in tile.responses:
                tile.responses[model_id].content = content
                tile.responses[model_id].status = status
                if status == "completed":
                    tile.responses[model_id].completed_at = datetime.now(timezone.utc)
                # Note: Don't save on every chunk - use save_session() after streaming
                return True

        return False

    def set_response_error(
        self, session_id: str, tile_id: str, model_id: str, error_message: str
    ) -> bool:
        """Set error state for a model response"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        for tile in session.prompt_tiles:
            if tile.id == tile_id and model_id in tile.responses:
                tile.responses[model_id].status = "error"
                tile.responses[model_id].error_message = error_message
                return True

        return False

    def save_session(self, session_id: str):
        """Explicitly save a session (call after streaming completes)"""
        session = self._sessions.get(session_id)
        if session:
            session.updated_at = datetime.now(timezone.utc)
            self._save(session)

    def add_debate(
        self,
        session_id: str,
        source_tile_ids: List[str],
        participating_models: List[str],
        debate_mode: DebateMode = DebateMode.AUTO,
        position: Optional[TilePosition] = None,
    ) -> Optional[DebateRound]:
        """Add a new debate to a session"""
        session = self._sessions.get(session_id)
        if not session:
            return None

        debate_id = str(uuid.uuid4())

        # Calculate position if not provided
        if position is None:
            base_x = 100 + len(session.debates) * 50
            base_y = 500 + len(session.debates) * 50
            position = TilePosition(x=base_x, y=base_y, width=600, height=400)

        debate = DebateRound(
            id=debate_id,
            participating_models=participating_models,
            debate_mode=debate_mode,
            source_tile_ids=source_tile_ids,
            position=position,
        )

        session.debates.append(debate)
        session.updated_at = datetime.now(timezone.utc)
        self._save(session)

        logger.info(f"Added debate {debate_id} to session {session_id}")
        return debate

    def add_debate_round(
        self, session_id: str, debate_id: str, responses: Dict[str, str]
    ) -> bool:
        """Add a round of responses to a debate"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        for debate in session.debates:
            if debate.id == debate_id:
                debate.rounds.append(responses)
                session.updated_at = datetime.now(timezone.utc)
                self._save(session)
                return True

        return False

    def update_debate_status(
        self, session_id: str, debate_id: str, status: str
    ) -> bool:
        """Update debate status (active, paused, completed)"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        for debate in session.debates:
            if debate.id == debate_id:
                debate.status = status
                session.updated_at = datetime.now(timezone.utc)
                self._save(session)
                return True

        return False

    def get_tile_responses(
        self, session_id: str, tile_ids: List[str]
    ) -> Dict[str, Dict[str, str]]:
        """Get responses from specified tiles (for debate context)"""
        session = self._sessions.get(session_id)
        if not session:
            return {}

        result = {}
        for tile in session.prompt_tiles:
            if tile.id in tile_ids:
                result[tile.id] = {
                    "prompt": tile.prompt,
                    "responses": {
                        model_id: response.content
                        for model_id, response in tile.responses.items()
                        if response.status == "completed"
                    },
                }

        return result

    def update_viewport(
        self, session_id: str, viewport: CanvasViewport
    ) -> bool:
        """Update canvas viewport state"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        session.viewport = viewport
        session.updated_at = datetime.now(timezone.utc)
        self._save(session)
        return True

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
    TileEdge,
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
        parent_tile_id: Optional[str] = None,
        parent_model_id: Optional[str] = None,
    ) -> Optional[PromptTile]:
        """Add a new prompt tile to a session"""
        session = self._sessions.get(session_id)
        if not session:
            return None

        tile_id = str(uuid.uuid4())

        # Calculate position if not provided
        if position is None:
            if parent_tile_id:
                # Position relative to parent tile for branching
                parent_tile = next(
                    (t for t in session.prompt_tiles if t.id == parent_tile_id), None
                )
                if parent_tile:
                    # Count existing children of this parent to offset
                    child_count = sum(
                        1 for t in session.prompt_tiles if t.parent_tile_id == parent_tile_id
                    )
                    base_x = parent_tile.position.x + parent_tile.position.width + 100
                    base_y = parent_tile.position.y + child_count * 350
                    position = TilePosition(x=base_x, y=base_y)
                else:
                    position = TilePosition(x=50, y=50)
            else:
                # Default grid layout for root tiles
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
            parent_tile_id=parent_tile_id,
            parent_model_id=parent_model_id,
        )

        session.prompt_tiles.append(tile)
        session.updated_at = datetime.now(timezone.utc)
        self._save(session)

        logger.info(f"Added prompt tile {tile_id} to session {session_id}" + 
                    (f" (branching from {parent_tile_id})" if parent_tile_id else ""))
        return tile

    def delete_tile(self, session_id: str, tile_id: str) -> bool:
        """Delete a tile (prompt or debate) from a session"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        # Try to delete from prompt tiles
        original_count = len(session.prompt_tiles)
        session.prompt_tiles = [t for t in session.prompt_tiles if t.id != tile_id]
        
        if len(session.prompt_tiles) < original_count:
            # Also remove any child tiles that reference this as parent
            session.prompt_tiles = [
                t for t in session.prompt_tiles 
                if t.parent_tile_id != tile_id
            ]
            session.updated_at = datetime.now(timezone.utc)
            self._save(session)
            logger.info(f"Deleted prompt tile {tile_id} from session {session_id}")
            return True

        # Try to delete from debates
        original_debate_count = len(session.debates)
        session.debates = [d for d in session.debates if d.id != tile_id]
        
        if len(session.debates) < original_debate_count:
            session.updated_at = datetime.now(timezone.utc)
            self._save(session)
            logger.info(f"Deleted debate {tile_id} from session {session_id}")
            return True

        return False

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

    def link_note(self, session_id: str, note_id: str) -> bool:
        """Link an exported note to this canvas session"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        session.linked_note_id = note_id
        session.updated_at = datetime.now(timezone.utc)
        self._save(session)
        logger.info(f"Linked note {note_id} to canvas session {session_id}")
        return True

    def get_tile_edges(self, session_id: str) -> List[TileEdge]:
        """Get all parent-child relationships for rendering mind-map edges"""
        session = self._sessions.get(session_id)
        if not session:
            return []

        edges = []
        for tile in session.prompt_tiles:
            if tile.parent_tile_id:
                edges.append(
                    TileEdge(
                        source_tile_id=tile.parent_tile_id,
                        target_tile_id=tile.id,
                        source_model_id=tile.parent_model_id,
                    )
                )
        return edges

    # ============================================================
    # Conversation Context Methods
    # ============================================================

    def build_full_history(
        self,
        session_id: str,
        parent_tile_id: Optional[str],
        parent_model_id: Optional[str],
    ) -> List[Dict[str, str]]:
        """
        Build conversation history by walking up the parent chain.
        Returns messages in chronological order (oldest first).

        Each turn includes:
        - user: the prompt from that tile
        - assistant: the response from the model that was used to branch from it
        """
        session = self._sessions.get(session_id)
        if not session or not parent_tile_id or not parent_model_id:
            return []

        # Collect turns walking backwards
        turns = []
        current_tile_id = parent_tile_id
        current_model_id = parent_model_id

        while current_tile_id:
            tile = next(
                (t for t in session.prompt_tiles if t.id == current_tile_id),
                None
            )
            if not tile:
                break

            # Get the specific model's response
            response = tile.responses.get(current_model_id)
            if not response or response.status != "completed":
                # Skip if response not available, but continue walking
                break

            turns.append({
                "prompt": tile.prompt,
                "response": response.content,
            })

            # Move to parent - use the tile's parent_model_id for next iteration
            current_tile_id = tile.parent_tile_id
            current_model_id = tile.parent_model_id
            if not current_model_id and tile.parent_tile_id:
                # Edge case: old tiles without parent_model_id
                break

        # Convert to messages format (oldest first)
        messages = []
        for turn in reversed(turns):
            messages.append({"role": "user", "content": turn["prompt"]})
            messages.append({"role": "assistant", "content": turn["response"]})

        return messages

    def build_compact_history(
        self,
        session_id: str,
        parent_tile_id: Optional[str],
        parent_model_id: Optional[str],
        max_recent: int = 2,
    ) -> List[Dict[str, str]]:
        """
        Build compact conversation history.
        Recent turns are included verbatim, older turns are summarized.

        Args:
            session_id: The canvas session ID
            parent_tile_id: The parent tile to start from
            parent_model_id: The model whose response to follow
            max_recent: Number of recent turns to include verbatim (default 2)

        Returns:
            List of messages with optional summary prefix
        """
        full_history = self.build_full_history(
            session_id, parent_tile_id, parent_model_id
        )

        # If history is short, return as-is
        if len(full_history) <= max_recent * 2:  # Each turn = 2 messages
            return full_history

        # Split into old and recent
        recent_start = len(full_history) - (max_recent * 2)
        old_turns = full_history[:recent_start]
        recent_turns = full_history[recent_start:]

        # Summarize old turns as context prefix
        # Extract prompts (every other message starting at 0)
        topics = []
        for i in range(0, len(old_turns), 2):
            prompt_content = old_turns[i]["content"]
            # Truncate long prompts
            if len(prompt_content) > 100:
                prompt_content = prompt_content[:100] + "..."
            topics.append(prompt_content)

        summary = f"Previous discussion covered: {'; '.join(topics)}"

        return [{"role": "system", "content": summary}] + recent_turns

    def build_semantic_context(
        self,
        prompt: str,
        vector_search,
        top_k: int = 3,
    ) -> List[Dict[str, str]]:
        """
        Build context by searching notes and tiles for relevant content.

        Args:
            prompt: The new prompt to find context for
            vector_search: VectorSearchService instance
            top_k: Number of results to include

        Returns:
            List with a single system message containing relevant context
        """
        if not vector_search:
            return []

        results = vector_search.search_all(prompt, limit=top_k)
        if not results:
            return []

        context_parts = []
        for r in results:
            if r.get("tile_id"):  # Canvas tile result
                context_parts.append(
                    f"Previous conversation:\nQ: {r['prompt']}\nA: {r['response']}"
                )
            else:  # Note result
                context_parts.append(
                    f"From note '{r['title']}':\n{r['snippet']}"
                )

        context = "Relevant context:\n\n" + "\n\n---\n\n".join(context_parts)
        return [{"role": "system", "content": context}]

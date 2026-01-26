"""Canvas session storage and persistence"""
import json
import logging
from pathlib import Path
from typing import List, Optional, Dict
from datetime import datetime, timezone
import uuid

from app.models.canvas import (
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
    NodeEdge,
    EdgeType,
)
from app.config import get_settings

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
        """Add a new prompt tile to a session with individual LLM node positioning"""
        session = self._sessions.get(session_id)
        if not session:
            return None

        tile_id = str(uuid.uuid4())

        # Per-model color palette (vibrant, distinct colors)
        MODEL_COLORS = [
            "#7c5cff",  # Violet
            "#22d3ee",  # Cyan
            "#f59e0b",  # Amber
            "#10b981",  # Emerald
            "#f43f5e",  # Rose
            "#8b5cf6",  # Purple
            "#06b6d4",  # Teal
            "#ec4899",  # Pink
            "#84cc16",  # Lime
            "#3b82f6",  # Blue
        ]

        # Calculate prompt node position if not provided
        if position is None:
            if parent_tile_id and parent_model_id:
                # Position relative to parent LLM node for branching
                parent_tile = next(
                    (t for t in session.prompt_tiles if t.id == parent_tile_id), None
                )
                if parent_tile and parent_model_id in parent_tile.responses:
                    parent_llm = parent_tile.responses[parent_model_id]
                    # Position to the right of the parent LLM node
                    base_x = parent_llm.position.x + parent_llm.position.width + 80
                    base_y = parent_llm.position.y
                    position = TilePosition(x=base_x, y=base_y, width=200, height=120)
                else:
                    position = TilePosition(x=50, y=50, width=200, height=120)
            else:
                # Default layout for root tiles - horizontal arrangement
                root_tiles = [t for t in session.prompt_tiles if not t.parent_tile_id]
                base_x = 50
                base_y = 50 + len(root_tiles) * 300  # Stack root prompts vertically
                position = TilePosition(x=base_x, y=base_y, width=200, height=120)

        # Create response placeholders with individual positions
        responses = {}
        for idx, model_id in enumerate(models):
            response_id = str(uuid.uuid4())
            model_name = model_id.split("/")[-1] if "/" in model_id else model_id
            
            # Position LLM nodes to the right of prompt, stacked vertically
            llm_x = position.x + position.width + 80  # Gap from prompt node
            llm_y = position.y + (idx * 220)  # Vertical stacking
            
            # Assign color based on model index (cycles through palette)
            color = MODEL_COLORS[idx % len(MODEL_COLORS)]
            
            responses[model_id] = ModelResponse(
                id=response_id,
                model_id=model_id,
                model_name=model_name,
                status="pending",
                position=TilePosition(x=llm_x, y=llm_y, width=280, height=200),
                color=color,
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

    def add_models_to_tile(
        self,
        session_id: str,
        tile_id: str,
        new_model_ids: List[str],
    ) -> bool:
        """Add new model responses to an existing tile (same prompt, new models)"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        # Find the tile
        tile = None
        for t in session.prompt_tiles:
            if t.id == tile_id:
                tile = t
                break

        if not tile:
            return False

        # Per-model color palette
        MODEL_COLORS = [
            "#7c5cff",  # Violet
            "#22d3ee",  # Cyan
            "#f59e0b",  # Amber
            "#10b981",  # Emerald
            "#f43f5e",  # Rose
            "#8b5cf6",  # Purple
            "#06b6d4",  # Teal
            "#ec4899",  # Pink
            "#84cc16",  # Lime
            "#3b82f6",  # Blue
        ]

        # Get existing response count for positioning
        existing_count = len(tile.responses)

        for idx, model_id in enumerate(new_model_ids):
            if model_id in tile.responses:
                # Skip if model already exists in this tile
                continue

            response_id = str(uuid.uuid4())
            model_name = model_id.split("/")[-1] if "/" in model_id else model_id
            
            # Position new LLM nodes below existing ones
            llm_x = tile.position.x + tile.position.width + 80
            llm_y = tile.position.y + ((existing_count + idx) * 220)
            
            # Assign color (continue from where existing models left off)
            color = MODEL_COLORS[(existing_count + idx) % len(MODEL_COLORS)]
            
            tile.responses[model_id] = ModelResponse(
                id=response_id,
                model_id=model_id,
                model_name=model_name,
                status="pending",
                position=TilePosition(x=llm_x, y=llm_y, width=280, height=200),
                color=color,
            )

            # Add to tile's model list
            if model_id not in tile.models:
                tile.models.append(model_id)

        session.updated_at = datetime.now(timezone.utc)
        # Don't save here - let the streaming complete and call save_session
        
        logger.info(f"Added {len(new_model_ids)} new models to tile {tile_id}")
        return True

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

    def update_llm_node_position(
        self, session_id: str, tile_id: str, model_id: str, position: TilePositionUpdate
    ) -> bool:
        """Update an individual LLM response node's position on the canvas"""
        session = self._sessions.get(session_id)
        if not session:
            return False

        for tile in session.prompt_tiles:
            if tile.id == tile_id and model_id in tile.responses:
                tile.responses[model_id].position.x = position.x
                tile.responses[model_id].position.y = position.y
                if position.width is not None:
                    tile.responses[model_id].position.width = position.width
                if position.height is not None:
                    tile.responses[model_id].position.height = position.height
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
        priority_scoring=None,
        knowledge_store=None,
    ) -> List[Dict[str, str]]:
        """
        Build context by searching notes and tiles for relevant content.
        Applies priority scoring to rank results by relevance.

        Args:
            prompt: The new prompt to find context for
            vector_search: VectorSearchService instance
            top_k: Number of results to include
            priority_scoring: Optional PriorityScoringService instance for prioritization
            knowledge_store: Optional KnowledgeStore instance for metadata

        Returns:
            List with a single system message containing relevant context
        """
        if not vector_search:
            return []

        # Get initial search results (fetch more for scoring)
        initial_limit = top_k * 3 if priority_scoring else top_k
        results = vector_search.search_all(prompt, limit=initial_limit)
        if not results:
            return []

        # Apply priority scoring if available
        if priority_scoring:
            # Extract tags from prompt for relevance matching
            from app.services.vector_search import ParsedQuery
            parsed = vector_search.parse_search_query(prompt)
            query_tags = parsed.tags
            
            # Score and sort results
            results = priority_scoring.score_search_results(
                results, query_tags, knowledge_store
            )
        
        # Take top_k results after scoring
        results = results[:top_k]

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

    # ============================================================
    # Node Graph Methods
    # ============================================================

    def find_node_groups(self, session_id: str) -> List[List[str]]:
        """
        Find connected components (isolated node groups) in the canvas.
        Returns list of groups, where each group is a list of node IDs.
        Node ID format: "prompt:{id}", "llm:{tile_id}:{model_id}", "debate:{id}"
        """
        session = self._sessions.get(session_id)
        if not session:
            return []

        # Build adjacency list
        adjacency: Dict[str, set] = {}

        def add_edge(node1: str, node2: str):
            if node1 not in adjacency:
                adjacency[node1] = set()
            if node2 not in adjacency:
                adjacency[node2] = set()
            adjacency[node1].add(node2)
            adjacency[node2].add(node1)

        # Process prompt tiles
        for tile in session.prompt_tiles:
            prompt_node = f"prompt:{tile.id}"
            if prompt_node not in adjacency:
                adjacency[prompt_node] = set()

            # Connect prompt to its LLM responses
            for model_id in tile.responses:
                llm_node = f"llm:{tile.id}:{model_id}"
                add_edge(prompt_node, llm_node)

            # Connect to parent LLM if branched
            if tile.parent_tile_id and tile.parent_model_id:
                parent_llm = f"llm:{tile.parent_tile_id}:{tile.parent_model_id}"
                add_edge(prompt_node, parent_llm)

        # Process debates
        for debate in session.debates:
            debate_node = f"debate:{debate.id}"
            if debate_node not in adjacency:
                adjacency[debate_node] = set()

            # Connect debate to source tile LLMs
            for source_tile_id in debate.source_tile_ids:
                source_tile = next(
                    (t for t in session.prompt_tiles if t.id == source_tile_id), None
                )
                if source_tile:
                    for model_id in debate.participating_models:
                        if model_id in source_tile.responses:
                            llm_node = f"llm:{source_tile_id}:{model_id}"
                            add_edge(debate_node, llm_node)

        # Find connected components using BFS
        visited: set = set()
        groups: List[List[str]] = []

        for node in adjacency:
            if node not in visited:
                component = []
                queue = [node]
                while queue:
                    current = queue.pop(0)
                    if current not in visited:
                        visited.add(current)
                        component.append(current)
                        queue.extend(adjacency[current] - visited)
                if component:
                    groups.append(component)

        return groups

    def batch_update_positions(
        self, session_id: str, positions: Dict[str, TilePositionUpdate]
    ) -> bool:
        """
        Batch update positions for multiple nodes (used by auto-arrange).
        positions dict keys use format: "prompt:{id}", "llm:{tile_id}:{model_id}", "debate:{id}"
        """
        session = self._sessions.get(session_id)
        if not session:
            return False

        updated = False

        for node_id, pos in positions.items():
            parts = node_id.split(":")

            if parts[0] == "prompt" and len(parts) >= 2:
                tile_id = parts[1]
                for tile in session.prompt_tiles:
                    if tile.id == tile_id:
                        tile.position.x = pos.x
                        tile.position.y = pos.y
                        if pos.width:
                            tile.position.width = pos.width
                        if pos.height:
                            tile.position.height = pos.height
                        updated = True
                        break

            elif parts[0] == "llm" and len(parts) >= 3:
                tile_id = parts[1]
                model_id = ":".join(parts[2:])  # Handle model IDs with colons
                for tile in session.prompt_tiles:
                    if tile.id == tile_id and model_id in tile.responses:
                        tile.responses[model_id].position.x = pos.x
                        tile.responses[model_id].position.y = pos.y
                        if pos.width:
                            tile.responses[model_id].position.width = pos.width
                        if pos.height:
                            tile.responses[model_id].position.height = pos.height
                        updated = True
                        break

            elif parts[0] == "debate" and len(parts) >= 2:
                debate_id = parts[1]
                for debate in session.debates:
                    if debate.id == debate_id:
                        debate.position.x = pos.x
                        debate.position.y = pos.y
                        if pos.width:
                            debate.position.width = pos.width
                        if pos.height:
                            debate.position.height = pos.height
                        updated = True
                        break

        if updated:
            session.updated_at = datetime.now(timezone.utc)
            self._save(session)

        return updated

    def get_node_edges(self, session_id: str) -> List[NodeEdge]:
        """Get all edges in the canvas graph for visualization"""
        session = self._sessions.get(session_id)
        if not session:
            return []

        edges = []

        # Prompt → LLM edges
        for tile in session.prompt_tiles:
            prompt_node = f"prompt:{tile.id}"
            for model_id, response in tile.responses.items():
                llm_node = f"llm:{tile.id}:{model_id}"
                edges.append(NodeEdge(
                    source_id=prompt_node,
                    target_id=llm_node,
                    edge_type=EdgeType.PROMPT_TO_LLM,
                    color=response.color,
                ))

            # LLM → Prompt branch edges
            if tile.parent_tile_id and tile.parent_model_id:
                parent_llm = f"llm:{tile.parent_tile_id}:{tile.parent_model_id}"
                edges.append(NodeEdge(
                    source_id=parent_llm,
                    target_id=prompt_node,
                    edge_type=EdgeType.LLM_TO_PROMPT,
                ))

        # Debate → LLM edges (conceptual - debates connect to the LLMs they originated from)
        for debate in session.debates:
            debate_node = f"debate:{debate.id}"
            for source_tile_id in debate.source_tile_ids:
                source_tile = next(
                    (t for t in session.prompt_tiles if t.id == source_tile_id), None
                )
                if source_tile:
                    for model_id in debate.participating_models:
                        if model_id in source_tile.responses:
                            llm_node = f"llm:{source_tile_id}:{model_id}"
                            edges.append(NodeEdge(
                                source_id=llm_node,
                                target_id=debate_node,
                                edge_type=EdgeType.DEBATE_TO_LLM,
                            ))

        return edges


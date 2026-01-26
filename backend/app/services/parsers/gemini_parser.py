"""Parser for Gemini conversation exports"""

import json
import logging
import re
from typing import List, Dict, Any, Optional, Tuple
from datetime import datetime, timezone

from app.services.parsers.base_parser import BaseParser
from app.models.import_models import (
    ParsedConversation,
    ParsedMessage,
    ConversationMetadata,
    AtomicNoteTemplate,
)

logger = logging.getLogger(__name__)


class GeminiParser(BaseParser):
    """Parser for Gemini export format"""

    def __init__(self):
        super().__init__()
        self.platform = "gemini"

    def detect_format(self, file_path: str) -> bool:
        """Check if file matches Gemini export format."""
        try:
            content = self._read_file(file_path)
            data = json.loads(content)

            # Check for Gemini-specific structure
            if isinstance(data, dict):
                # Gemini export has project + messages or conversation array
                return any(
                    key in data
                    for key in [
                        "project",
                        "conversation_id",
                        "gemini_version",
                        "canvas_content",
                        "chat_messages",
                        "messages",  # Combined export
                    ]
                )
            elif isinstance(data, list):
                # Array of messages
                if len(data) == 0:
                    return False
                first_item = data[0]
                if isinstance(first_item, dict):
                    return any(
                        key in first_item
                        for key in ["role", "content", "message", "id", "timestamp"]
                    )

            return False
        except (json.JSONDecodeError, FileNotFoundError):
            return False

    async def parse(self, file_path: str) -> List[ParsedConversation]:
        """
        Parse Gemini export file.

        Expected format (from Gemini userscript):
        {
            "project": "Project Name",
            "conversation_id": "...",
            "export_time": "2025-09-09 14:30:22",
            "messages": [
                { "role": "user", "content": "...", "id": 0 },
                { "role": "model", "content": "...", "id": 1, "model": "gemini-pro" }
            ],
            "canvas_content": [ ... ]  // Optional: exported canvas
        }

        Or combined export (chat + canvas)
        """
        content = self._read_file(file_path)
        data = json.loads(content)

        conversations: List[ParsedConversation] = []

        # Handle single conversation with metadata
        if isinstance(data, dict) and ("messages" in data or "chat_messages" in data):
            conv = self._parse_single_conversation(data)
            if conv:
                conversations.append(conv)

        # Handle array of conversations
        elif isinstance(data, list):
            for conv_data in data:
                try:
                    conv = self._parse_single_conversation(conv_data)
                    if conv:
                        conversations.append(conv)
                except Exception as e:
                    logger.error(f"Failed to parse Gemini conversation: {e}")

        logger.info(f"Parsed {len(conversations)} Gemini conversations")
        return conversations

    def _parse_single_conversation(
        self, data: Dict[str, Any]
    ) -> Optional[ParsedConversation]:
        """Parse a single Gemini conversation."""
        conv_id = data.get("conversation_id", data.get("id", "unknown"))
        project_name = data.get("project", "Untitled Project")

        # Build title from project name
        title = f"Gemini: {project_name}"

        # Extract messages
        messages_key = self._find_messages_key(data)
        messages_list = data.get(messages_key, [])
        messages = self._extract_messages(messages_list)

        if not messages:
            logger.warning(f"No messages found in Gemini conversation: {conv_id}")
            return None

        # Extract model info
        model_info = []
        for msg in messages:
            if msg.model and msg.model not in model_info:
                model_info.append(msg.model)

        # Parse timestamps
        export_time = data.get("export_time", data.get("timestamp"))
        created_at = self._parse_timestamp(export_time)
        updated_at = created_at

        # Check for canvas content
        canvas_content = data.get("canvas_content", [])

        # Build metadata
        metadata = ConversationMetadata(
            platform="gemini",
            source_url=data.get("url"),
            created_at=created_at,
            updated_at=updated_at,
            message_count=len(messages),
            model_info=sorted(model_info) or ["gemini"],
            platform_specific={
                "project_name": project_name,
                "conversation_id": conv_id,
                "export_time": export_time,
                "has_canvas": len(canvas_content) > 0,
                "canvas_items": len(canvas_content),
            },
        )

        conv = ParsedConversation(
            id=conv_id,
            title=title,
            platform="gemini",
            messages=messages,
            metadata=metadata,
            suggested_tags=self._suggest_tags(project_name, messages),
        )

        return conv

    def _find_messages_key(self, data: Dict[str, Any]) -> str:
        """Find key that contains messages in Gemini export."""
        possible_keys = ["messages", "chat_messages", "conversation", "chat"]
        for key in possible_keys:
            if key in data and isinstance(data[key], list):
                return key
        return "messages"

    def _extract_messages(
        self, messages_list: List[Dict[str, Any]]
    ) -> List[ParsedMessage]:
        """
        Extract messages from Gemini messages array.

        Gemini message format:
        {
            "role": "user" | "model",
            "content": "text content..." | { "parts": [...] },
            "model": "gemini-pro",  // only for model responses
            "timestamp": "2025-09-09 14:30:22",
            "id": 0
        }
        """
        messages: List[ParsedMessage] = []

        for i, msg_data in enumerate(messages_list):
            # Determine role
            role = msg_data.get("role", "")

            # Map Gemini roles to standard ones
            if role == "user":
                std_role = "user"
            elif role == "model":
                std_role = "assistant"
            else:
                # Try to infer from index
                std_role = "user" if i % 2 == 0 else "assistant"

            # Extract content
            content = self._extract_message_content(msg_data)

            if not content:
                continue

            # Parse timestamp
            timestamp = self._parse_timestamp(msg_data.get("timestamp"))

            # Extract model (only for assistant messages)
            model = None
            if std_role == "assistant":
                model = msg_data.get("model", "gemini")

            parsed_msg = ParsedMessage(
                index=i,
                role=std_role,
                content=content,
                timestamp=timestamp,
                model=model,
                metadata={"original_role": role, "original_index": i},
            )

            messages.append(parsed_msg)

        return messages

    def _extract_message_content(self, msg_data: Dict[str, Any]) -> str:
        """
        Extract text content from Gemini message.

        Content can be:
        - String: Simple text
        - Object with parts array (multimodal content)
        """
        content_obj = msg_data.get("content")

        if not content_obj:
            return ""

        # Simple string
        if isinstance(content_obj, str):
            return content_obj

        # Object with parts
        if isinstance(content_obj, dict):
            parts = content_obj.get("parts", [])
            if not parts:
                return ""

            text_parts = []
            for part in parts:
                # Text part
                if isinstance(part, str):
                    text_parts.append(part)
                elif isinstance(part, dict):
                    # Look for text field
                    if "text" in part:
                        text_parts.append(part["text"])
                    # Handle inline code blocks
                    elif "code" in part:
                        text_parts.append(
                            f"```{part.get('language', '')}\n{part['code']}\n```"
                        )

            return "\n".join(text_parts)

        return str(content_obj)

    def _parse_timestamp(self, timestamp: Any) -> Optional[datetime]:
        """Parse Gemini timestamp to datetime."""
        if not timestamp:
            return None

        if isinstance(timestamp, (int, float)):
            return datetime.fromtimestamp(timestamp, tz=timezone.utc)

        if isinstance(timestamp, str):
            try:
                # Gemini uses various timestamp formats
                # Try ISO format first
                timestamp = timestamp.replace("T", " ").replace("Z", "")
                return datetime.fromisoformat(timestamp)
            except ValueError:
                # Try simple date format
                for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"):
                    try:
                        return datetime.strptime(timestamp, fmt)
                    except ValueError:
                        continue
                return None

        return None

    def _suggest_tags(
        self, project_name: str, messages: List[ParsedMessage]
    ) -> List[str]:
        """Suggest tags based on conversation content."""
        tags = ["gemini", "import"]

        # Extract keywords from project name
        name_lower = project_name.lower()

        # Gemini-specific keywords
        gemini_keywords = {
            "coding": ["python", "javascript", "code", "programming", "api"],
            "writing": ["write", "essay", "article", "content"],
            "analysis": ["analyze", "explain", "compare", "understand"],
            "multimodal": ["image", "vision", "code", "canvas"],
            "creative": ["creative", "idea", "brainstorm", "generate"],
        }

        for category, keywords in gemini_keywords.items():
            if any(keyword in name_lower for keyword in keywords):
                tags.append(category)
                break

        return tags[:5]

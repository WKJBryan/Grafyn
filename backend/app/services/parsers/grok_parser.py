"""Parser for Grok conversation exports"""

import json
import logging
from typing import List, Dict, Any, Optional
from datetime import datetime, timezone

from app.services.parsers.base_parser import BaseParser
from app.models.import_models import (
    ParsedConversation,
    ParsedMessage,
    ConversationMetadata,
)

logger = logging.getLogger(__name__)


class GrokParser(BaseParser):
    """Parser for Grok (xAI) export format"""

    def __init__(self):
        super().__init__()
        self.platform = "grok"

    def detect_format(self, file_path: str) -> bool:
        """Check if file matches Grok export format."""
        try:
            content = self._read_file(file_path)
            data = json.loads(content)

            # Check for Grok-specific keys
            if isinstance(data, dict):
                # Grok export from userscript has metadata + chats array
                return any(
                    key in data
                    for key in [
                        "meta",
                        "speaker_stats",
                        "chats",
                        "conversation",
                        "grok_mode",
                        "platform",  # Enhanced Grok Export format
                    ]
                )
            elif isinstance(data, list):
                # Array of messages/conversations
                if len(data) == 0:
                    return False
                first_item = data[0]
                if isinstance(first_item, dict):
                    return any(
                        key in first_item
                        for key in ["type", "message", "index", "mode"]
                    )

            return False
        except (json.JSONDecodeError, FileNotFoundError):
            return False

    async def parse(self, file_path: str) -> List[ParsedConversation]:
        """
        Parse Grok export file.

        Expected format (from Enhanced Grok Export userscript):
        {
            "meta": { "exported_at": "...", "title": "..." },
            "chats": [
                { "index": 0, "type": "prompt", "message": "..." },
                { "index": 1, "type": "response", "message": "...", "mode": "think" }
            ],
            "speaker_stats": { "human": 40, "grok": 60 }
        }

        Or decoder format (after hex decode):
        Array of conversations
        """
        content = self._read_file(file_path)
        data = json.loads(content)

        conversations: List[ParsedConversation] = []

        # Handle Enhanced Grok Export format (meta + chats)
        if isinstance(data, dict) and "meta" in data:
            conv = self._parse_enhanced_format(data)
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
                    logger.error(f"Failed to parse Grok conversation: {e}")

        logger.info(f"Parsed {len(conversations)} Grok conversations")
        return conversations

    def _parse_enhanced_format(
        self, data: Dict[str, Any]
    ) -> Optional[ParsedConversation]:
        """Parse Enhanced Grok Export format (meta + chats)."""
        meta = data.get("meta", {})
        title = meta.get("title", "Grok Conversation")
        exported_at = meta.get("exported_at")

        # Extract messages
        chats = data.get("chats", [])
        messages = self._extract_messages_from_enhanced(chats)

        if not messages:
            logger.warning("No messages found in Grok export")
            return None

        # Extract mode info from messages
        mode_info = set()
        for msg in messages:
            if msg.metadata and msg.metadata.get("mode"):
                mode_info.add(msg.metadata["mode"])

        # Parse timestamp
        created_at = self._parse_timestamp(exported_at)
        updated_at = created_at

        # Build metadata
        metadata = ConversationMetadata(
            platform="grok",
            source_url=meta.get("source_url"),
            created_at=created_at,
            updated_at=updated_at,
            message_count=len(messages),
            model_info=["grok"],  # Grok uses one model
            platform_specific={
                "exported_at": exported_at,
                "speaker_stats": data.get("speaker_stats", {}),
                "modes_used": sorted(mode_info),
                "think_mode_detected": "think" in mode_info,
                "fun_mode_detected": "fun" in mode_info,
                "deepsearch_mode_detected": "deepsearch" in mode_info,
            },
        )

        return ParsedConversation(
            id=f"grok_{exported_at or 'unknown'}",
            title=title,
            platform="grok",
            messages=messages,
            metadata=metadata,
            suggested_tags=self._suggest_tags(title, messages, list(mode_info)),
        )

    def _extract_messages_from_enhanced(
        self, chats: List[Dict[str, Any]]
    ) -> List[ParsedMessage]:
        """
        Extract messages from Enhanced Grok Export format.

        Chat message format:
        {
            "index": 0,
            "type": "prompt" | "response",
            "message": { "type": "p", "data": "text..." },
            "mode": "think" | "fun" | "deepsearch"  (optional)
        }
        """
        messages: List[ParsedMessage] = []

        for chat in chats:
            msg_type = chat.get("type", "")
            index = chat.get("index", len(messages))

            # Determine role
            if msg_type == "prompt":
                role = "user"
            elif msg_type == "response":
                role = "assistant"
            else:
                # Try to infer from index
                role = "user" if index % 2 == 0 else "assistant"

            # Extract content
            content = self._extract_content_from_enhanced(chat.get("message", {}))

            if not content:
                continue

            # Extract mode (for assistant messages)
            mode = chat.get("mode")
            timestamp = self._parse_timestamp(chat.get("timestamp"))

            parsed_msg = ParsedMessage(
                index=index,
                role=role,
                content=content,
                timestamp=timestamp,
                model="grok",
                metadata={"type": msg_type, "mode": mode, "original_index": index},
            )

            messages.append(parsed_msg)

        return messages

    def _extract_content_from_enhanced(self, message_obj: Dict[str, Any]) -> str:
        """
        Extract text from Enhanced Grok Export message object.

        Message can be:
        - String: Direct text
        - Object with "data" field containing text
        - Array of parts (paragraphs, code blocks, etc.)
        """
        if isinstance(message_obj, str):
            return message_obj

        if not isinstance(message_obj, dict):
            return ""

        # Check for data field
        if "data" in message_obj:
            data = message_obj["data"]
            if isinstance(data, str):
                return data
            elif isinstance(data, list):
                # Array of content parts
                text_parts = []
                for part in data:
                    if isinstance(part, str):
                        text_parts.append(part)
                    elif isinstance(part, dict):
                        # Look for text content
                        if "data" in part and isinstance(part["data"], str):
                            text_parts.append(part["data"])
                return "\n".join(text_parts)

        # Direct content field
        if "content" in message_obj:
            return str(message_obj["content"])

        return ""

    def _parse_single_conversation(
        self, data: Dict[str, Any]
    ) -> Optional[ParsedConversation]:
        """Parse a single Grok conversation object."""
        # Generic fallback for other Grok export formats
        conv_id = data.get("id", "unknown")
        title = data.get("title", "Grok Conversation")

        messages = self._extract_messages_generic(data)

        if not messages:
            return None

        created_at = self._parse_timestamp(data.get("created_at"))
        updated_at = self._parse_timestamp(data.get("updated_at")) or created_at

        metadata = ConversationMetadata(
            platform="grok",
            source_url=data.get("url"),
            created_at=created_at,
            updated_at=updated_at,
            message_count=len(messages),
            model_info=["grok"],
            platform_specific={},
        )

        return ParsedConversation(
            id=conv_id,
            title=title,
            platform="grok",
            messages=messages,
            metadata=metadata,
            suggested_tags=self._suggest_tags(title, messages, []),
        )

    def _extract_messages_generic(self, data: Dict[str, Any]) -> List[ParsedMessage]:
        """Extract messages from generic Grok format."""
        messages: List[ParsedMessage] = []
        messages_list = data.get("messages", data.get("chats", []))

        for i, msg_data in enumerate(messages_list):
            # Determine role
            role = msg_data.get("role", "")
            if role not in ("user", "assistant", "system"):
                # Try to infer from type
                msg_type = msg_data.get("type", "")
                if msg_type == "prompt":
                    role = "user"
                elif msg_type == "response":
                    role = "assistant"
                else:
                    role = "user" if i % 2 == 0 else "assistant"

            content = msg_data.get("content", msg_data.get("message", ""))
            if not content:
                continue

            timestamp = self._parse_timestamp(msg_data.get("timestamp"))
            mode = msg_data.get("mode")

            parsed_msg = ParsedMessage(
                index=i,
                role=role,
                content=str(content),
                timestamp=timestamp,
                model="grok",
                metadata={"mode": mode} if mode else {},
            )

            messages.append(parsed_msg)

        return messages

    def _parse_timestamp(self, timestamp: Any) -> Optional[datetime]:
        """Parse Grok timestamp to datetime."""
        if not timestamp:
            return None

        if isinstance(timestamp, (int, float)):
            return datetime.fromtimestamp(timestamp, tz=timezone.utc)

        if isinstance(timestamp, str):
            try:
                # Handle various formats
                timestamp = timestamp.replace("T", " ").replace("Z", "")
                return datetime.fromisoformat(timestamp)
            except ValueError:
                return None

        return None

    def _suggest_tags(
        self, title: str, messages: List[ParsedMessage], modes: List[str]
    ) -> List[str]:
        """Suggest tags based on conversation content and modes."""
        tags = ["grok", "import"]

        # Add mode-specific tags
        if "think" in modes:
            tags.append("think-mode")
        if "fun" in modes:
            tags.append("fun-mode")
        if "deepsearch" in modes:
            tags.append("deepsearch")

        # Extract keywords from title
        title_lower = title.lower()
        grok_keywords = {
            "coding": ["python", "javascript", "code", "programming"],
            "research": ["research", "analyze", "find", "search"],
            "creative": ["creative", "fun", "joke", "story"],
            "social": ["x", "twitter", "post", "tweet"],
        }

        for category, keywords in grok_keywords.items():
            if any(keyword in title_lower for keyword in keywords):
                tags.append(category)
                break

        return tags[:5]

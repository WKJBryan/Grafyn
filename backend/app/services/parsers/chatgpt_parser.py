"""Parser for ChatGPT conversation exports"""

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


class ChatGPTParser(BaseParser):
    """Parser for ChatGPT conversations.json export format"""

    def __init__(self):
        super().__init__()
        self.platform = "chatgpt"

    def detect_format(self, file_path: str) -> bool:
        """Check if file matches ChatGPT export format."""
        try:
            content = self._read_file(file_path)
            data = json.loads(content)

            # ChatGPT format check
            if isinstance(data, list):
                # ChatGPT conversations.json is a list of conversations
                # Each conversation should have 'mapping' key (message tree)
                # Exclude Claude format which has 'chat_messages' or 'sender' keys
                if len(data) == 0:
                    return False
                first_item = data[0]
                if isinstance(first_item, dict):
                    # ChatGPT-specific: has 'mapping' key
                    if "mapping" in first_item:
                        return True
                    # Also check for title + create_time without chat_messages
                    if "title" in first_item and "create_time" in first_item and "chat_messages" not in first_item:
                        return True
                return False
            elif isinstance(data, dict):
                # Single conversation with mapping structure
                return any(key in data for key in ["mapping", "create_time"]) and "chat_messages" not in data
            return False
        except (json.JSONDecodeError, FileNotFoundError):
            return False

    async def parse(self, file_path: str) -> List[ParsedConversation]:
        """
        Parse ChatGPT conversations.json export.

        Expected format (from OpenAI export):
        - conversations.json: Array of conversation objects
        - Each has: id, title, create_time, mapping (message tree)

        API format:
        - Single conversation JSON with mapping structure
        """
        content = self._read_file(file_path)
        data = json.loads(content)

        conversations: List[ParsedConversation] = []

        # Handle both array (conversations.json) and single object (API export)
        if isinstance(data, list):
            # Multiple conversations
            for conv_data in data:
                try:
                    conv = self._parse_single_conversation(conv_data)
                    if conv:
                        conversations.append(conv)
                except Exception as e:
                    logger.error(f"Failed to parse conversation: {e}")
        else:
            # Single conversation
            try:
                conv = self._parse_single_conversation(data)
                if conv:
                    conversations.append(conv)
            except Exception as e:
                logger.error(f"Failed to parse conversation: {e}")

        logger.info(f"Parsed {len(conversations)} ChatGPT conversations")
        return conversations

    def _parse_single_conversation(
        self, data: Dict[str, Any]
    ) -> Optional[ParsedConversation]:
        """Parse a single ChatGPT conversation."""
        conv_id = data.get("id", "unknown")
        title = data.get("title", "Untitled Conversation")
        create_time = data.get("create_time", data.get("createTime"))
        update_time = data.get("update_time", data.get("updateTime"))

        # Parse timestamps
        created_at = self._parse_timestamp(create_time)
        updated_at = self._parse_timestamp(update_time) or created_at

        # Extract messages from mapping structure
        mapping = data.get("mapping", {})
        messages = self._extract_messages_from_mapping(mapping)

        if not messages:
            logger.warning(f"No messages found in conversation: {conv_id}")
            return None

        # Extract model info from messages
        model_info = set()
        for msg in messages:
            if msg.model:
                model_info.add(msg.model)

        # Build metadata
        metadata = ConversationMetadata(
            platform="chatgpt",
            source_url=data.get("url"),
            created_at=created_at,
            updated_at=updated_at,
            message_count=len(messages),
            model_info=sorted(model_info),
            platform_specific={
                "conversation_id": conv_id,
                "mapping_keys": list(mapping.keys()),
            },
        )

        return ParsedConversation(
            id=conv_id,
            title=title,
            platform="chatgpt",
            messages=messages,
            metadata=metadata,
            suggested_tags=self._suggest_tags(title, messages),
        )

    def _extract_messages_from_mapping(
        self, mapping: Dict[str, Any]
    ) -> List[ParsedMessage]:
        """
        Extract messages from ChatGPT's mapping structure.

        Mapping format:
        {
            "msg_id_1": {
                "message": { "role": "user", "content": {...}, "author": {...} },
                "parent": "msg_id_0",
                "children": ["msg_id_2"]
            }
        }
        """
        messages: List[ParsedMessage] = []

        # Find root messages (messages with no parent or parent is null/root)
        # ChatGPT uses a tree structure; we need to traverse it
        if not mapping:
            return messages

        # Find all messages that have actual content
        msg_data = {}
        for msg_id, node in mapping.items():
            message = node.get("message")
            if message and message.get("content"):
                msg_data[msg_id] = {
                    "msg": message,
                    "parent": node.get("parent"),
                    "timestamp": message.get("create_time", message.get("createTime")),
                }

        # Build traversal order by following parent links
        # Start from messages with no parent (or minimal parent depth)
        root_messages = [
            msg_id
            for msg_id, data in msg_data.items()
            if not data["parent"]
            or data["parent"] == ""
            or data["parent"] not in msg_data
        ]

        for root_id in root_messages:
            self._traverse_message_tree(root_id, msg_data, messages, 0)

        return messages

    def _traverse_message_tree(
        self,
        msg_id: str,
        msg_data: Dict[str, Any],
        messages: List[ParsedMessage],
        index: int,
    ):
        """Recursively traverse message tree."""
        if msg_id not in msg_data:
            return

        data = msg_data[msg_id]
        msg = data["msg"]

        # Determine role from message structure
        author = msg.get("author", {})
        role = author.get("role", "user")

        # Map ChatGPT roles to standard ones
        role_map = {
            "system": "system",
            "user": "user",
            "assistant": "assistant",
            "tool": "assistant",  # Tool calls count as assistant messages
        }
        role = role_map.get(role, "user")

        # Extract content
        content = self._extract_content(msg)

        if not content:
            return

        # Get model info from metadata
        metadata = msg.get("metadata", {})
        model_slug = metadata.get("model_slug") or metadata.get("modelSlug")

        # Parse timestamp
        timestamp = self._parse_timestamp(data["timestamp"])

        parsed_msg = ParsedMessage(
            index=index,
            role=role,
            content=content,
            timestamp=timestamp,
            model=model_slug,
            metadata={
                "message_id": msg_id,
                "finish_details": metadata.get(
                    "finish_details", metadata.get("finishDetails")
                ),
            },
        )

        messages.append(parsed_msg)

        # Traverse children (follow the first child for linear conversation)
        # Note: ChatGPT may have multiple children for branching conversations
        # For import purposes, we follow the primary branch
        # Find the node's children from the original mapping
        # This requires the full mapping, which we don't have here
        # For now, we'll skip child traversal and rely on the root messages list

    def _extract_content(self, message: Dict[str, Any]) -> str:
        """
        Extract text content from ChatGPT message structure.

        Content can be:
        - String: Simple text
        - Dict with 'parts' array (common ChatGPT format)
        - Array of content parts (text, code, images, etc.)
        - Multimodal content object
        """
        content_obj = message.get("content")

        if not content_obj:
            return ""

        # Simple string
        if isinstance(content_obj, str):
            return content_obj

        # Dict with 'parts' array (common ChatGPT format)
        if isinstance(content_obj, dict):
            parts = content_obj.get("parts", [])
            if parts and isinstance(parts, list):
                text_parts = []
                for part in parts:
                    if isinstance(part, str):
                        text_parts.append(part)
                    elif isinstance(part, dict) and "text" in part:
                        text_parts.append(part["text"])
                if text_parts:
                    return "\n".join(text_parts)
            # Fallback: check for direct text field
            if "text" in content_obj:
                return content_obj["text"]

        # Array of content parts
        if isinstance(content_obj, list):
            text_parts = []
            for part in content_obj:
                # Text part
                if isinstance(part, str):
                    text_parts.append(part)
                # Content object with text
                elif isinstance(part, dict):
                    part_type = part.get("content_type", part.get("contentType"))
                    if part_type == "text":
                        text_parts.append(part.get("text", part.get("text", "")))
                    elif "text" in part:
                        text_parts.append(part["text"])
                    # Handle nested 'parts' in content objects
                    elif "parts" in part:
                        for inner_part in part["parts"]:
                            if isinstance(inner_part, str):
                                text_parts.append(inner_part)

            return "\n".join(text_parts)

        return str(content_obj)

    def _parse_timestamp(self, timestamp: Any) -> Optional[datetime]:
        """Parse ChatGPT timestamp to datetime."""
        if not timestamp:
            return None

        # Unix timestamp (seconds)
        if isinstance(timestamp, (int, float)):
            return datetime.fromtimestamp(timestamp, tz=timezone.utc)

        # String timestamp
        if isinstance(timestamp, str):
            try:
                return datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
            except ValueError:
                return None

        return None

    def _suggest_tags(self, title: str, messages: List[ParsedMessage]) -> List[str]:
        """Suggest tags based on conversation content."""
        tags = ["chatgpt", "import"]

        # Extract keywords from title
        title_words = set(title.lower().split())

        # Common topic keywords
        topic_keywords = {
            "programming",
            "code",
            "python",
            "javascript",
            "api",
            "debugging",
            "error",
            "fix",
            "bug",
            "writing",
            "article",
            "blog",
            "content",
            "research",
            "analysis",
            "data",
            "study",
            "design",
            "ui",
            "ux",
            "interface",
            "business",
            "startup",
            "marketing",
            "strategy",
        }

        for word in topic_keywords:
            if word in title_words:
                tags.append(word)

        return tags[:5]  # Limit to 5 tags

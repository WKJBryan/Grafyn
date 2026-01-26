"""Parser for Claude conversation exports"""

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


class ClaudeParser(BaseParser):
    """Parser for Claude export format (.dms or JSON)"""

    def __init__(self):
        super().__init__()
        self.platform = "claude"

    def detect_format(self, file_path: str) -> bool:
        """Check if file matches Claude export format."""
        try:
            content = self._read_file(file_path)
            data = json.loads(content)

            # Check for Claude-specific structure
            if isinstance(data, dict):
                # Claude export often has conversation metadata
                return any(
                    key in data
                    for key in [
                        "uuid",
                        "chat",
                        "conversation",
                        "model",
                        "chat_log",
                        "conversation_log",
                    ]
                )
            elif isinstance(data, list):
                # Array of conversations
                if len(data) == 0:
                    return False
                first_item = data[0]
                if isinstance(first_item, dict):
                    # Check for Claude-specific keys including chat_messages
                    return any(
                        key in first_item
                        for key in ["uuid", "chat", "conversation", "message", "chat_messages"]
                    )

            return False
        except (json.JSONDecodeError, FileNotFoundError):
            return False

    async def parse(self, file_path: str) -> List[ParsedConversation]:
        """
        Parse Claude export file.

        Expected format (from Claude web export or userscript):
        - JSON with conversations array or single conversation
        - Each has: uuid, model, name/title, messages array
        - Messages: {type, content, model} where type is prompt/response
        """
        content = self._read_file(file_path)
        data = json.loads(content)

        conversations: List[ParsedConversation] = []

        # Handle both array and single object
        if isinstance(data, list):
            for conv_data in data:
                try:
                    conv = self._parse_single_conversation(conv_data)
                    if conv:
                        conversations.append(conv)
                except Exception as e:
                    logger.error(f"Failed to parse Claude conversation: {e}")
        else:
            try:
                conv = self._parse_single_conversation(data)
                if conv:
                    conversations.append(conv)
            except Exception as e:
                logger.error(f"Failed to parse Claude conversation: {e}")

        logger.info(f"Parsed {len(conversations)} Claude conversations")
        return conversations

    def _parse_single_conversation(
        self, data: Dict[str, Any]
    ) -> Optional[ParsedConversation]:
        """Parse a single Claude conversation."""
        conv_id = data.get("uuid", data.get("id", "unknown"))
        title = data.get("name", data.get("title", "Untitled Conversation"))

        # Claude exports may have multiple message key names
        messages_key = self._find_messages_key(data)
        messages_list = data.get(messages_key, [])

        # Parse messages
        messages = self._extract_messages(messages_list)

        if not messages:
            logger.warning(f"No messages found in Claude conversation: {conv_id}")
            return None

        # Extract model info
        model = data.get("model", "unknown")
        model_info = []
        if model and model != "unknown":
            model_info.append(model)

        # Also check individual messages for models (may vary in conversation)
        for msg in messages:
            if msg.model and msg.model not in model_info:
                model_info.append(msg.model)

        # Parse timestamps (Claude may use different formats)
        created_at = self._parse_timestamp(
            data.get("created_at", data.get("createdAt"))
        )
        updated_at = (
            self._parse_timestamp(data.get("updated_at", data.get("updatedAt")))
            or created_at
        )

        # Build metadata
        metadata = ConversationMetadata(
            platform="claude",
            source_url=data.get("url"),
            created_at=created_at,
            updated_at=updated_at,
            message_count=len(messages),
            model_info=sorted(model_info),
            platform_specific={
                "uuid": conv_id,
                "claude_version": data.get("claude_version"),
                "messages_key": messages_key,
            },
        )

        return ParsedConversation(
            id=conv_id,
            title=title,
            platform="claude",
            messages=messages,
            metadata=metadata,
            suggested_tags=self._suggest_tags(title, messages),
        )

    def _find_messages_key(self, data: Dict[str, Any]) -> str:
        """Find the key that contains messages in Claude export."""
        possible_keys = [
            "chat_messages",  # Official Claude export format
            "messages",
            "chat",
            "conversation",
            "chat_log",
            "conversation_log",
        ]
        for key in possible_keys:
            if key in data and isinstance(data[key], list):
                return key
        return "messages"

    def _extract_messages(
        self, messages_list: List[Dict[str, Any]]
    ) -> List[ParsedMessage]:
        """
        Extract messages from Claude messages array.

        Claude message format:
        {
            "type": "prompt" | "response",
            "content": { "type": "text", "text": "..." },
            "model": "claude-3-opus-20240229",
            "timestamp": "2024-01-01T12:00:00Z"
        }

        Or simpler format from userscript:
        {
            "index": 0,
            "type": "prompt" | "response",
            "message": "text content..."
        }
        """
        messages: List[ParsedMessage] = []

        for i, msg_data in enumerate(messages_list):
            # Determine message type from 'type' or 'sender' field
            msg_type = msg_data.get("type", "")
            sender = msg_data.get("sender", "")
            
            if msg_type == "prompt" or sender == "human":
                role = "user"
            elif msg_type == "response" or sender == "assistant":
                role = "assistant"
            else:
                # Try to infer from index (even = user, odd = assistant)
                role = "user" if i % 2 == 0 else "assistant"

            # Extract content
            content = self._extract_message_content(msg_data)

            if not content:
                continue

            # Parse timestamp - check multiple possible field names
            timestamp = self._parse_timestamp(
                msg_data.get("timestamp") or 
                msg_data.get("created_at") or 
                msg_data.get("updated_at")
            )

            # Extract model (only in responses)
            model = None
            if role == "assistant":
                model = msg_data.get("model", msg_data.get("model_id"))

            parsed_msg = ParsedMessage(
                index=i,
                role=role,
                content=content,
                timestamp=timestamp,
                model=model,
                metadata={"type": msg_type or sender, "original_index": i},
            )

            messages.append(parsed_msg)

        return messages

    def _extract_message_content(self, msg_data: Dict[str, Any]) -> str:
        """Extract text content from Claude message."""
        # Check for direct text field (official Claude export format)
        if "text" in msg_data:
            text = msg_data["text"]
            if isinstance(text, str):
                return text
        
        # Check for direct message field
        if "message" in msg_data:
            msg = msg_data["message"]
            if isinstance(msg, str):
                return msg
            elif isinstance(msg, dict) and "text" in msg:
                return msg["text"]

        # Check for content field
        if "content" in msg_data:
            content = msg_data["content"]

            # String content
            if isinstance(content, str):
                return content

            # Object with text
            elif isinstance(content, dict):
                if content.get("type") == "text":
                    return content.get("text", "")
                elif "text" in content:
                    return content["text"]

            # Array of content parts
            elif isinstance(content, list):
                text_parts = []
                for part in content:
                    if isinstance(part, str):
                        text_parts.append(part)
                    elif isinstance(part, dict):
                        if part.get("type") == "text":
                            text_parts.append(part.get("text", ""))
                        elif "text" in part:
                            text_parts.append(part["text"])
                return "\n".join(text_parts)

        return ""

    def _parse_timestamp(self, timestamp: Any) -> Optional[datetime]:
        """Parse Claude timestamp to datetime."""
        if not timestamp:
            return None

        # Unix timestamp
        if isinstance(timestamp, (int, float)):
            return datetime.fromtimestamp(timestamp, tz=timezone.utc)

        # ISO 8601 string
        if isinstance(timestamp, str):
            try:
                return datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
            except ValueError:
                return None

        return None

    def _suggest_tags(self, title: str, messages: List[ParsedMessage]) -> List[str]:
        """Suggest tags based on conversation content."""
        tags = ["claude", "import"]

        # Extract keywords from title
        title_lower = title.lower()

        # Claude-specific keywords
        claude_keywords = {
            "coding": ["python", "javascript", "code", "programming", "debug"],
            "writing": ["write", "essay", "article", "blog"],
            "analysis": ["analyze", "compare", "explain", "understand"],
            "creative": ["creative", "story", "poem", "idea", "brainstorm"],
            "technical": ["technical", "api", "documentation", "troubleshoot"],
        }

        for category, keywords in claude_keywords.items():
            if any(keyword in title_lower for keyword in keywords):
                tags.append(category)
                break

        return tags[:5]

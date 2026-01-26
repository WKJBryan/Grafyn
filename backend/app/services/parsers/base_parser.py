"""Base parser interface for LLM conversation exports"""

from abc import ABC, abstractmethod
from typing import List, Optional
from pathlib import Path

from app.models.import_models import ParsedConversation


class BaseParser(ABC):
    """Base class for all LLM conversation parsers"""

    def __init__(self):
        self.platform = "unknown"

    @abstractmethod
    async def parse(self, file_path: str) -> List[ParsedConversation]:
        """
        Parse export file and return list of conversations.

        Args:
            file_path: Path to the export file

        Returns:
            List of parsed conversations

        Raises:
            ValueError: If file format is invalid
        """
        pass

    @abstractmethod
    def detect_format(self, file_path: str) -> bool:
        """
        Check if file matches this parser's format.

        Args:
            file_path: Path to the export file

        Returns:
            True if file matches this parser's format
        """
        pass

    def _read_file(self, file_path: str) -> str:
        """Read file content as string."""
        path = Path(file_path)
        if not path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        with open(path, "r", encoding="utf-8") as f:
            return f.read()

    def _validate_messages(self, messages: List) -> int:
        """Validate messages count."""
        return len(messages)

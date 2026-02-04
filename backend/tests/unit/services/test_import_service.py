"""Unit tests for ImportService"""
import json
import os
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from app.models.import_models import (
    ConversationMetadata,
    ImportDecision,
    ImportJob,
    ParsedConversation,
    ParsedMessage,
    PreviewResult,
)
from app.services.import_service import ImportService


# ============================================================================
# Helpers
# ============================================================================

def _make_parsed_conversation(
    conv_id="conv-1",
    title="Test Conversation",
    platform="chatgpt",
    messages=None,
    suggested_tags=None,
):
    """Build a minimal ParsedConversation for testing."""
    if messages is None:
        messages = [
            ParsedMessage(
                index=0,
                role="user",
                content="How do I use pandas?",
                timestamp=datetime(2024, 1, 1, tzinfo=timezone.utc),
            ),
            ParsedMessage(
                index=1,
                role="assistant",
                content="You can use pd.read_csv() to load CSV files.",
                timestamp=datetime(2024, 1, 1, 0, 1, tzinfo=timezone.utc),
                model="gpt-4",
            ),
        ]
    return ParsedConversation(
        id=conv_id,
        title=title,
        platform=platform,
        messages=messages,
        metadata=ConversationMetadata(
            platform=platform,
            created_at=datetime(2024, 1, 1, tzinfo=timezone.utc),
            updated_at=datetime(2024, 1, 1, 0, 1, tzinfo=timezone.utc),
            message_count=len(messages),
            model_info=["gpt-4"],
        ),
        suggested_tags=suggested_tags or ["chatgpt", "import"],
    )


def _chatgpt_export():
    """Return a minimal ChatGPT conversations.json structure (single conv)."""
    return [
        {
            "title": "Python Data Analysis Discussion",
            "create_time": 1704067200,
            "update_time": 1704153600,
            "mapping": {
                "msg-1": {
                    "id": "msg-1",
                    "message": {
                        "id": "msg-1",
                        "author": {"role": "user"},
                        "content": {
                            "parts": ["How do I analyze CSV data with pandas?"]
                        },
                        "create_time": 1704067200,
                    },
                    "parent": None,
                    "children": ["msg-2"],
                },
                "msg-2": {
                    "id": "msg-2",
                    "message": {
                        "id": "msg-2",
                        "author": {
                            "role": "assistant",
                            "metadata": {"model_slug": "gpt-4"},
                        },
                        "content": {
                            "parts": [
                                "To analyze CSV data with pandas, you can use pd.read_csv()..."
                            ]
                        },
                        "create_time": 1704067260,
                    },
                    "parent": "msg-1",
                    "children": [],
                },
            },
        }
    ]


def _write_chatgpt_file(tmp_path, data=None):
    """Write a ChatGPT export JSON file and return its path."""
    file_path = tmp_path / "conversations.json"
    file_path.write_text(json.dumps(data or _chatgpt_export()), encoding="utf-8")
    return file_path


# ============================================================================
# upload_file
# ============================================================================


@pytest.mark.unit
class TestUploadFile:
    """Tests for ImportService.upload_file"""

    @pytest.mark.asyncio
    async def test_upload_creates_job_with_uploaded_status(self, import_service):
        """upload_file should return a job with status 'uploaded'."""
        content = b'{"hello": "world"}'
        job = await import_service.upload_file(content, "test.json")

        assert job.status == "uploaded"
        assert job.file_name == "test.json"
        assert job.id in import_service.jobs

    @pytest.mark.asyncio
    async def test_upload_saves_file_to_temp_dir(self, import_service):
        """upload_file should persist the raw bytes on disk."""
        content = b"raw file bytes"
        job = await import_service.upload_file(content, "export.json")

        saved_path = Path(job.file_path)
        assert saved_path.exists()
        assert saved_path.read_bytes() == content

    @pytest.mark.asyncio
    async def test_upload_sets_timestamps(self, import_service):
        """Job should have created_at and updated_at timestamps."""
        job = await import_service.upload_file(b"data", "f.json")

        assert job.created_at is not None
        assert job.updated_at is not None
        assert job.created_at.tzinfo is not None  # timezone-aware

    @pytest.mark.asyncio
    async def test_upload_generates_unique_ids(self, import_service):
        """Each upload should produce a distinct job ID."""
        job1 = await import_service.upload_file(b"a", "a.json")
        job2 = await import_service.upload_file(b"b", "b.json")

        assert job1.id != job2.id


# ============================================================================
# parse_file
# ============================================================================


@pytest.mark.unit
class TestParseFile:
    """Tests for ImportService.parse_file"""

    @pytest.mark.asyncio
    async def test_parse_raises_for_unknown_job_id(self, import_service):
        """parse_file should raise ValueError for a nonexistent job."""
        with pytest.raises(ValueError, match="Job not found"):
            await import_service.parse_file("nonexistent-id")

    @pytest.mark.asyncio
    async def test_parse_succeeds_with_chatgpt_file(
        self, import_service, tmp_path
    ):
        """parse_file should detect the ChatGPT parser and set status to 'parsed'."""
        file_path = _write_chatgpt_file(tmp_path)
        content = file_path.read_bytes()
        job = await import_service.upload_file(content, "conversations.json")

        # Overwrite the saved file with valid ChatGPT JSON so the parser can
        # read it from the path stored in the job.
        Path(job.file_path).write_text(
            json.dumps(_chatgpt_export()), encoding="utf-8"
        )

        result = await import_service.parse_file(job.id)

        assert result.status == "parsed"
        assert result.platform == "chatgpt"
        assert result.total_conversations >= 1
        assert result.parsed_conversations is not None

    @pytest.mark.asyncio
    async def test_parse_fails_for_unrecognised_format(self, import_service):
        """parse_file should set status to 'failed' when no parser matches."""
        content = b"this is plain text, not JSON"
        job = await import_service.upload_file(content, "random.txt")

        result = await import_service.parse_file(job.id)

        assert result.status == "failed"
        assert len(result.errors) > 0
        assert result.errors[0].type == "parse_error"

    @pytest.mark.asyncio
    async def test_parse_sets_platform_on_job(self, import_service, tmp_path):
        """After successful parse the job.platform should be set."""
        file_path = _write_chatgpt_file(tmp_path)
        content = file_path.read_bytes()
        job = await import_service.upload_file(content, "conversations.json")
        Path(job.file_path).write_text(
            json.dumps(_chatgpt_export()), encoding="utf-8"
        )

        result = await import_service.parse_file(job.id)
        assert result.platform == "chatgpt"


# ============================================================================
# get_preview
# ============================================================================


@pytest.mark.unit
class TestGetPreview:
    """Tests for ImportService.get_preview"""

    @pytest.mark.asyncio
    async def test_preview_raises_for_unknown_job(self, import_service):
        """get_preview should raise ValueError for a nonexistent job."""
        with pytest.raises(ValueError, match="Job not found"):
            await import_service.get_preview("does-not-exist")

    @pytest.mark.asyncio
    async def test_preview_raises_for_wrong_status(self, import_service):
        """get_preview requires the job to be in 'parsed' or 'reviewing' status."""
        job = await import_service.upload_file(b"data", "f.json")
        # job.status is still 'uploaded'
        with pytest.raises(ValueError, match="not ready for preview"):
            await import_service.get_preview(job.id)

    @pytest.mark.asyncio
    async def test_preview_returns_stats(self, import_service, tmp_path):
        """get_preview should return a PreviewResult with correct stats."""
        # Prepare a parsed job
        file_path = _write_chatgpt_file(tmp_path)
        content = file_path.read_bytes()
        job = await import_service.upload_file(content, "conversations.json")
        Path(job.file_path).write_text(
            json.dumps(_chatgpt_export()), encoding="utf-8"
        )
        await import_service.parse_file(job.id)

        preview = await import_service.get_preview(job.id)

        assert isinstance(preview, PreviewResult)
        assert preview.job_id == job.id
        assert preview.total_conversations >= 1
        assert preview.estimated_notes_to_create > 0
        assert preview.platform == "chatgpt"

    @pytest.mark.asyncio
    async def test_preview_raises_when_no_conversations(self, import_service):
        """get_preview should raise when parsed_conversations is empty."""
        job = await import_service.upload_file(b"x", "x.json")
        # Manually set status to parsed but leave parsed_conversations empty
        job.status = "parsed"
        job.parsed_conversations = None

        with pytest.raises(ValueError, match="No conversations parsed"):
            await import_service.get_preview(job.id)


# ============================================================================
# apply_import
# ============================================================================


@pytest.mark.unit
class TestApplyImport:
    """Tests for ImportService.apply_import"""

    @pytest.mark.asyncio
    async def test_apply_raises_for_unknown_job(self, import_service):
        """apply_import should raise ValueError for a nonexistent job."""
        with pytest.raises(ValueError, match="Job not found"):
            await import_service.apply_import("bad-id", [])

    @pytest.mark.asyncio
    async def test_apply_skip_increments_counter(self, import_service):
        """A decision with action='skip' should increment skipped count."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [ImportDecision(conversation_id="c1", action="skip")]
        summary = await import_service.apply_import(job.id, decisions)

        assert summary.skipped == 1
        assert summary.imported == 0
        assert summary.total_conversations == 1

    @pytest.mark.asyncio
    async def test_apply_accept_creates_container_note(
        self, import_service, knowledge_store
    ):
        """A decision with action='accept' should create a container note."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [
            ImportDecision(
                conversation_id="c1",
                action="accept",
                distill_option="container_only",
            )
        ]
        summary = await import_service.apply_import(job.id, decisions)

        assert summary.imported == 1
        assert summary.container_notes == 1
        assert summary.notes_created >= 1

        # Verify note exists in knowledge store
        notes = knowledge_store.list_notes()
        assert len(notes) >= 1

    @pytest.mark.asyncio
    async def test_apply_merge_with_target(self, import_service, knowledge_store):
        """A decision with action='merge' should append to the target note."""
        # Create target note first
        from app.models.note import NoteCreate

        target = knowledge_store.create_note(
            NoteCreate(
                title="Existing Note",
                content="Original content.",
                tags=["test"],
                status="draft",
            )
        )

        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [
            ImportDecision(
                conversation_id="c1",
                action="merge",
                target_note_id=target.id,
            )
        ]
        summary = await import_service.apply_import(job.id, decisions)

        assert summary.merged == 1

        # Verify the note content was appended
        updated = knowledge_store.get_note(target.id)
        assert "Original content." in updated.content
        assert "Update from" in updated.content

    @pytest.mark.asyncio
    async def test_apply_skips_missing_conversation(self, import_service):
        """If conversation_id doesn't match any parsed conversation, skip it."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [
            ImportDecision(conversation_id="nonexistent", action="accept")
        ]
        summary = await import_service.apply_import(job.id, decisions)

        # The missing conversation is silently skipped
        assert summary.skipped == 1
        assert summary.imported == 0

    @pytest.mark.asyncio
    async def test_apply_sets_completed_status(self, import_service):
        """After apply_import the job status should be 'completed'."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [ImportDecision(conversation_id="c1", action="skip")]
        await import_service.apply_import(job.id, decisions)

        assert import_service.jobs[job.id].status == "completed"

    @pytest.mark.asyncio
    async def test_apply_calls_progress_callback(self, import_service):
        """progress_callback should be called during import."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        messages = []
        decisions = [ImportDecision(conversation_id="c1", action="skip")]
        await import_service.apply_import(
            job.id, decisions, progress_callback=messages.append
        )

        assert len(messages) >= 1


# ============================================================================
# revert_import
# ============================================================================


@pytest.mark.unit
class TestRevertImport:
    """Tests for ImportService.revert_import"""

    @pytest.mark.asyncio
    async def test_revert_raises_for_unknown_job(self, import_service, knowledge_store):
        """revert_import should raise ValueError when no notes are tracked."""
        with pytest.raises(ValueError, match="No import to revert"):
            await import_service.revert_import("bad-id", knowledge_store)

    @pytest.mark.asyncio
    async def test_revert_deletes_created_notes(
        self, import_service, knowledge_store
    ):
        """revert_import should delete notes that were created during import."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [
            ImportDecision(
                conversation_id="c1",
                action="accept",
                distill_option="container_only",
            )
        ]
        await import_service.apply_import(job.id, decisions)

        # Verify notes were created
        notes_before = knowledge_store.list_notes()
        assert len(notes_before) >= 1

        result = await import_service.revert_import(job.id, knowledge_store)

        assert result["deleted"] >= 1
        # After revert, the notes should be gone
        notes_after = knowledge_store.list_notes()
        assert len(notes_after) < len(notes_before)

    @pytest.mark.asyncio
    async def test_revert_clears_tracking(self, import_service, knowledge_store):
        """After revert, the job's created notes tracking should be removed."""
        conv = _make_parsed_conversation(conv_id="c1")
        job = await import_service.upload_file(b"x", "x.json")
        job.status = "parsed"
        job.parsed_conversations = [conv]

        decisions = [
            ImportDecision(
                conversation_id="c1",
                action="accept",
                distill_option="container_only",
            )
        ]
        await import_service.apply_import(job.id, decisions)

        await import_service.revert_import(job.id, knowledge_store)

        assert job.id not in import_service.job_created_notes


# ============================================================================
# Parser detection (_detect_parser)
# ============================================================================


@pytest.mark.unit
class TestParserDetection:
    """Tests for _detect_parser and ChatGPT format detection."""

    def test_detect_chatgpt_json(self, import_service, tmp_path):
        """_detect_parser should recognise a ChatGPT conversations.json file."""
        file_path = _write_chatgpt_file(tmp_path)
        parser = import_service._detect_parser(str(file_path))

        assert parser is not None
        assert parser.platform == "chatgpt"

    def test_detect_returns_none_for_plain_text(self, import_service, tmp_path):
        """_detect_parser should return None for an unrecognised format."""
        file_path = tmp_path / "notes.txt"
        file_path.write_text("just some plain text", encoding="utf-8")

        parser = import_service._detect_parser(str(file_path))
        assert parser is None

    def test_detect_returns_none_for_invalid_json(self, import_service, tmp_path):
        """_detect_parser should return None for malformed JSON."""
        file_path = tmp_path / "bad.json"
        file_path.write_text("{invalid json", encoding="utf-8")

        parser = import_service._detect_parser(str(file_path))
        assert parser is None

    def test_detect_returns_none_for_non_chatgpt_json(
        self, import_service, tmp_path
    ):
        """_detect_parser should return None for JSON that isn't a known format."""
        file_path = tmp_path / "other.json"
        file_path.write_text(
            json.dumps({"some_key": "some_value"}), encoding="utf-8"
        )

        parser = import_service._detect_parser(str(file_path))
        assert parser is None


# ============================================================================
# Error handling and edge cases
# ============================================================================


@pytest.mark.unit
class TestErrorHandling:
    """Tests for error handling and invalid state transitions."""

    @pytest.mark.asyncio
    async def test_cancel_job_removes_file_and_job(self, import_service):
        """cancel_job should delete the uploaded file and remove the job."""
        job = await import_service.upload_file(b"data", "f.json")
        assert Path(job.file_path).exists()

        result = import_service.cancel_job(job.id)

        assert result is True
        assert not Path(job.file_path).exists()
        assert job.id not in import_service.jobs

    @pytest.mark.asyncio
    async def test_cancel_unknown_job_returns_false(self, import_service):
        """cancel_job should return False for a nonexistent job."""
        assert import_service.cancel_job("nonexistent") is False

    @pytest.mark.asyncio
    async def test_find_conversation_returns_none_for_missing_id(
        self, import_service
    ):
        """_find_conversation should return None when conversation_id is absent."""
        job = ImportJob(
            id="j1",
            status="parsed",
            file_path="/tmp/f.json",
            file_name="f.json",
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
            parsed_conversations=[_make_parsed_conversation(conv_id="c1")],
        )
        result = import_service._find_conversation(job, "c999")
        assert result is None

    @pytest.mark.asyncio
    async def test_find_conversation_returns_match(self, import_service):
        """_find_conversation should return the matching conversation."""
        conv = _make_parsed_conversation(conv_id="target")
        job = ImportJob(
            id="j1",
            status="parsed",
            file_path="/tmp/f.json",
            file_name="f.json",
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
            parsed_conversations=[conv],
        )
        result = import_service._find_conversation(job, "target")
        assert result is not None
        assert result.id == "target"

    @pytest.mark.asyncio
    async def test_find_conversation_with_no_parsed(self, import_service):
        """_find_conversation should return None when parsed_conversations is None."""
        job = ImportJob(
            id="j1",
            status="parsed",
            file_path="/tmp/f.json",
            file_name="f.json",
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
            parsed_conversations=None,
        )
        result = import_service._find_conversation(job, "c1")
        assert result is None

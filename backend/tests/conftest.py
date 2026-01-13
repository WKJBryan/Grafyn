"""Pytest configuration and shared fixtures for Seedream backend tests"""
import os
import shutil
from pathlib import Path
from typing import Generator
from datetime import datetime, timedelta

import pytest
from fastapi.testclient import TestClient

from app.main import app
from app.config import Settings, get_settings
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.services.embedding import EmbeddingService
from app.services.token_store import TokenStore
from app.services.priority_scoring import PriorityScoringService, PriorityWeights
from app.services.priority_settings import PrioritySettingsService


# ============================================================================
# Test Settings and Configuration
# ============================================================================

@pytest.fixture(scope="session")
def embedding_service_session() -> EmbeddingService:
    """
    Session-scoped embedding service to avoid downloading model multiple times.
    The sentence-transformers model is downloaded once and cached.
    """
    return EmbeddingService()


@pytest.fixture
def temp_vault_path(tmp_path: Path) -> Path:
    """Create a temporary vault directory for testing"""
    vault_dir = tmp_path / "test_vault"
    vault_dir.mkdir(parents=True, exist_ok=True)
    return vault_dir


@pytest.fixture
def temp_data_path(tmp_path: Path) -> Path:
    """Create a temporary data directory for LanceDB"""
    data_dir = tmp_path / "test_data"
    data_dir.mkdir(parents=True, exist_ok=True)
    return data_dir


@pytest.fixture
def temp_token_storage_path(tmp_path: Path) -> Path:
    """Create a temporary directory for token storage"""
    token_dir = tmp_path / "test_tokens"
    token_dir.mkdir(parents=True, exist_ok=True)
    return token_dir


@pytest.fixture
def test_settings(temp_vault_path: Path, temp_data_path: Path) -> Settings:
    """Override settings for testing"""
    return Settings(
        server_host="127.0.0.1",
        server_port=8080,
        vault_path=str(temp_vault_path),
        data_path=str(temp_data_path),
        embedding_model="all-MiniLM-L6-v2",
        environment="testing",
        cors_origins="http://localhost:5173",
        rate_limit_enabled=False,  # Disable rate limiting in tests
        github_client_id="test_client_id",
        github_client_secret="test_client_secret",
        github_redirect_uri="http://localhost:8080/api/oauth/callback/github",
        token_encryption_key="test_encryption_key_32_bytes_long!",
    )


@pytest.fixture(autouse=True)
def override_get_settings(test_settings: Settings, monkeypatch):
    """Automatically override get_settings for all tests"""
    monkeypatch.setattr("app.config.get_settings", lambda: test_settings)
    monkeypatch.setattr("app.main.settings", test_settings)
    return test_settings


# ============================================================================
# Service Fixtures
# ============================================================================

@pytest.fixture
def knowledge_store(temp_vault_path: Path) -> KnowledgeStore:
    """Create a KnowledgeStore instance with temporary vault"""
    store = KnowledgeStore()
    store.vault_path = temp_vault_path
    return store


@pytest.fixture
def embedding_service(embedding_service_session: EmbeddingService) -> EmbeddingService:
    """Return the session-scoped embedding service"""
    return embedding_service_session


@pytest.fixture
def vector_search(temp_data_path: Path, embedding_service: EmbeddingService) -> Generator[VectorSearchService, None, None]:
    """Create a VectorSearchService with temporary data directory"""
    service = VectorSearchService()
    service.data_path = temp_data_path
    service.embedding_service = embedding_service
    service._initialize_db()
    yield service
    # Cleanup
    try:
        service.clear_all()
    except:
        pass


@pytest.fixture
def graph_index(knowledge_store: KnowledgeStore) -> GraphIndexService:
    """Create a GraphIndexService instance"""
    service = GraphIndexService()
    service.knowledge_store = knowledge_store
    return service


@pytest.fixture
def token_store(temp_token_storage_path: Path) -> TokenStore:
    """Create a TokenStore instance with temporary storage"""
    store = TokenStore(storage_dir=str(temp_token_storage_path))
    return store


# ============================================================================
# Test Client Fixtures
# ============================================================================

@pytest.fixture
def priority_scoring_service() -> PriorityScoringService:
    """Create a PriorityScoringService instance"""
    weights = PriorityWeights()
    return PriorityScoringService(weights)


@pytest.fixture
def priority_settings_service(temp_data_path: Path) -> PrioritySettingsService:
    """Create a PrioritySettingsService instance with temporary storage"""
    return PrioritySettingsService(temp_data_path / "priority_settings.json")


@pytest.fixture
def test_client(
    knowledge_store: KnowledgeStore,
    vector_search: VectorSearchService,
    graph_index: GraphIndexService,
    priority_scoring_service: PriorityScoringService,
    priority_settings_service: PrioritySettingsService,
) -> TestClient:
    """Create a FastAPI TestClient with all services attached"""
    # Attach services to app.state
    app.state.knowledge_store = knowledge_store
    app.state.vector_search = vector_search
    app.state.graph_index = graph_index
    app.state.priority_scoring = priority_scoring_service
    app.state.priority_settings = priority_settings_service

    return TestClient(app)


# ============================================================================
# Test Data Fixtures
# ============================================================================

@pytest.fixture
def sample_note_data() -> dict:
    """Sample note data for testing"""
    return {
        "title": "Test Note",
        "content": "This is a test note with some content.\n\nIt has multiple paragraphs.",
        "status": "draft",
        "tags": ["test", "sample"],
    }


@pytest.fixture
def sample_note_with_wikilinks() -> dict:
    """Sample note with wikilinks for graph testing"""
    return {
        "title": "Note With Links",
        "content": "This note links to [[Target Note]] and [[Another Note|with display text]].",
        "status": "draft",
        "tags": ["linked"],
    }


@pytest.fixture
def sample_notes_list() -> list[dict]:
    """List of sample notes for bulk testing"""
    return [
        {
            "title": "Python Programming",
            "content": "Python is a high-level programming language.\n\n[[JavaScript]] is another popular language.",
            "status": "canonical",
            "tags": ["programming", "python"],
        },
        {
            "title": "JavaScript",
            "content": "JavaScript is the language of the web.\n\nSee also: [[Python Programming]] and [[Web Development]].",
            "status": "canonical",
            "tags": ["programming", "javascript", "web"],
        },
        {
            "title": "Web Development",
            "content": "Web development involves [[JavaScript]], HTML, and CSS.",
            "status": "evidence",
            "tags": ["web", "programming"],
        },
        {
            "title": "Machine Learning",
            "content": "Machine learning is a subset of AI. Popular with [[Python Programming]].",
            "status": "draft",
            "tags": ["ai", "python", "ml"],
        },
        {
            "title": "Database Design",
            "content": "Databases store and organize data efficiently.",
            "status": "canonical",
            "tags": ["database", "architecture"],
        },
    ]


@pytest.fixture
def create_sample_notes(knowledge_store: KnowledgeStore, sample_notes_list: list[dict]):
    """Create sample notes in the test vault"""
    created_ids = []
    for note_data in sample_notes_list:
        note = knowledge_store.create_note(note_data)
        created_ids.append(note["id"])
    return created_ids


@pytest.fixture
def sample_markdown_files(temp_vault_path: Path):
    """Create sample markdown files in the test vault"""
    notes = {
        "note-1": """---
title: First Note
status: draft
tags:
  - test
  - first
created_at: 2025-01-01T12:00:00Z
updated_at: 2025-01-01T12:00:00Z
---

# First Note

This is the first test note.

It links to [[note-2]] and [[note-3|Third Note]].
""",
        "note-2": """---
title: Second Note
status: canonical
tags:
  - test
  - second
created_at: 2025-01-02T12:00:00Z
updated_at: 2025-01-02T12:00:00Z
---

# Second Note

This is the second test note.

It mentions [[note-1]] and has some content.
""",
        "note-3": """---
title: Third Note
status: evidence
tags:
  - test
  - third
created_at: 2025-01-03T12:00:00Z
updated_at: 2025-01-03T12:00:00Z
---

# Third Note

This note doesn't link to anything.
""",
    }

    for note_id, content in notes.items():
        note_path = temp_vault_path / f"{note_id}.md"
        note_path.write_text(content, encoding="utf-8")

    return list(notes.keys())


# ============================================================================
# Security Test Fixtures
# ============================================================================

@pytest.fixture
def path_traversal_attempts() -> list[str]:
    """Common path traversal attack patterns for security testing"""
    return [
        "../../etc/passwd",
        "../../../etc/passwd",
        "..\\..\\..\\windows\\system32\\config\\sam",
        "....//....//....//etc/passwd",
        "..%2F..%2F..%2Fetc%2Fpasswd",
        "..%252F..%252F..%252Fetc%252Fpasswd",
        "..\\\\..\\\\..\\\\windows\\\\system32",
        "/etc/passwd",
        "C:\\Windows\\System32\\config\\sam",
        "./../.../../etc/passwd",
    ]


@pytest.fixture
def malicious_wikilink_patterns() -> list[str]:
    """Wikilink patterns that might cause issues"""
    return [
        "[[../../etc/passwd]]",
        "[[<script>alert('xss')</script>]]",
        "[[" + "A" * 10000 + "]]",  # Very long wikilink
        "[[]]",  # Empty wikilink
        "[[|display]]",  # Empty target
        "[[target|]]",  # Empty display
        "[[target|display|extra]]",  # Extra pipes
        "[[nested [[inner]] link]]",  # Nested wikilinks
    ]


# ============================================================================
# OAuth and Token Fixtures
# ============================================================================

@pytest.fixture
def oauth_state_token() -> str:
    """Generate a test OAuth state token"""
    import secrets
    return secrets.token_urlsafe(32)


@pytest.fixture
def oauth_code() -> str:
    """Generate a test OAuth authorization code"""
    import secrets
    return secrets.token_urlsafe(32)


@pytest.fixture
def valid_access_token() -> str:
    """Generate a test access token"""
    import secrets
    return secrets.token_urlsafe(32)


@pytest.fixture
def expired_token_data(token_store: TokenStore, valid_access_token: str) -> dict:
    """Create an expired token for testing"""
    import secrets
    token_id = secrets.token_urlsafe(16)
    expires_at = datetime.utcnow() - timedelta(hours=1)  # Expired 1 hour ago

    user_data = {
        "id": "test_user",
        "login": "testuser",
        "email": "test@example.com",
    }

    token_store.store_token(token_id, valid_access_token, user_data, expires_at)

    return {
        "token_id": token_id,
        "access_token": valid_access_token,
        "user_data": user_data,
        "expires_at": expires_at,
    }


# ============================================================================
# Performance and Load Testing Fixtures
# ============================================================================

@pytest.fixture
def large_note_dataset() -> list[dict]:
    """Generate a large dataset of notes for performance testing"""
    from faker import Faker
    fake = Faker()

    notes = []
    for i in range(100):
        content_paragraphs = [fake.paragraph() for _ in range(5)]
        content = "\n\n".join(content_paragraphs)

        # Add some random wikilinks
        if i > 0 and i % 3 == 0:
            linked_note = f"Note {i - 1}"
            content += f"\n\nSee also: [[{linked_note}]]"

        notes.append({
            "title": f"Note {i}",
            "content": content,
            "status": ["draft", "evidence", "canonical"][i % 3],
            "tags": [fake.word() for _ in range(3)],
        })

    return notes


# ============================================================================
# Utility Fixtures
# ============================================================================

@pytest.fixture
def freeze_time():
    """Freeze time for testing time-sensitive operations"""
    from freezegun import freeze_time as _freeze_time
    return _freeze_time


@pytest.fixture
def mock_github_oauth_response():
    """Mock GitHub OAuth response for testing"""
    return {
        "access_token": "gho_test_access_token_123456",
        "token_type": "bearer",
        "scope": "read:user,user:email",
    }


@pytest.fixture
def mock_github_user_response():
    """Mock GitHub user API response"""
    return {
        "id": 123456,
        "login": "testuser",
        "email": "testuser@example.com",
        "name": "Test User",
        "avatar_url": "https://avatars.githubusercontent.com/u/123456",
    }

# Seedream Backend Tests

Comprehensive test suite for the Seedream knowledge graph platform backend.

## Test Coverage Summary

### ✅ Completed (210+ tests)

#### Unit Tests - Services
- **test_knowledge_store.py** (50+ tests)
  - Path traversal protection (security-critical)
  - CRUD operations
  - Wikilink extraction and parsing
  - Frontmatter parsing
  - Unicode and character encoding
  - Edge cases and error handling

- **test_vector_search.py** (45+ tests)
  - LanceDB initialization
  - Single and batch indexing
  - Semantic search functionality
  - Vector dimension validation (384-dim)
  - Upsert behavior
  - Delete operations
  - Performance tests

- **test_graph_index.py** (35+ tests)
  - Graph index building
  - Outgoing links retrieval
  - Backlinks with context
  - Neighbor traversal (BFS, depth 1-3)
  - Unlinked mentions detection
  - Circular link handling
  - Incremental updates

- **test_token_store.py** (40+ tests)
  - Fernet encryption/decryption
  - Token storage and retrieval
  - Token expiration handling
  - File permission enforcement (0o600)
  - Concurrent access safety
  - CSRF state management
  - Security tests

- **test_embedding.py** (40+ tests)
  - Model loading (all-MiniLM-L6-v2)
  - Single text encoding
  - Batch encoding
  - Vector dimension validation
  - Unicode handling
  - Encoding consistency
  - Performance tests

### 🚧 To Be Implemented

#### Unit Tests - Middleware (Pending)
- test_security.py (20+ tests needed)
- test_rate_limit.py (15+ tests needed)
- test_logging.py (10+ tests needed)

#### Unit Tests - Models (Pending)
- test_note_models.py (20+ tests needed)

#### Integration Tests (Pending)
- test_notes_api.py (40+ tests needed)
- test_search_api.py (25+ tests needed)
- test_graph_api.py (30+ tests needed)
- test_oauth_api.py (20+ tests needed)
- test_full_workflow.py (15+ tests needed)

## Quick Start

### Install Dependencies

```bash
cd backend
pip install -r requirements.txt -r requirements-dev.txt
```

### Run All Tests

```bash
# Run all tests with coverage
pytest

# Run with verbose output
pytest -v

# Run with coverage report
pytest --cov=app --cov-report=html
```

### Run Specific Test Categories

```bash
# Run only unit tests
pytest -m unit

# Run only integration tests
pytest -m integration

# Run only security tests
pytest -m security

# Skip slow tests
pytest -m "not slow"
```

### Run Specific Test Files

```bash
# Test knowledge store
pytest tests/unit/services/test_knowledge_store.py

# Test vector search
pytest tests/unit/services/test_vector_search.py

# Test graph index
pytest tests/unit/services/test_graph_index.py

# Test token store
pytest tests/unit/services/test_token_store.py

# Test embedding service
pytest tests/unit/services/test_embedding.py
```

### Run Specific Test Classes or Functions

```bash
# Run a specific test class
pytest tests/unit/services/test_knowledge_store.py::TestPathTraversalProtection

# Run a specific test function
pytest tests/unit/services/test_knowledge_store.py::TestPathTraversalProtection::test_path_traversal_unix_style

# Run tests matching a pattern
pytest -k "traversal"
```

## Test Markers

Tests are organized with pytest markers:

- `@pytest.mark.unit` - Unit tests for individual components
- `@pytest.mark.integration` - Integration tests for API endpoints
- `@pytest.mark.security` - Security-focused tests
- `@pytest.mark.slow` - Slow-running tests (performance, large datasets)

## Test Fixtures

Core fixtures are defined in `conftest.py`:

### Service Fixtures
- `knowledge_store` - KnowledgeStore with temp vault
- `vector_search` - VectorSearchService with temp LanceDB
- `graph_index` - GraphIndexService
- `embedding_service` - EmbeddingService (session-scoped, shared)
- `token_store` - TokenStore with temp storage

### Path Fixtures
- `temp_vault_path` - Temporary vault directory
- `temp_data_path` - Temporary data directory for LanceDB
- `temp_token_storage_path` - Temporary token storage

### Settings Fixtures
- `test_settings` - Override settings for testing
- `override_get_settings` - Auto-applied settings override

### Data Fixtures
- `sample_note_data` - Single sample note
- `sample_note_with_wikilinks` - Note with wikilinks
- `sample_notes_list` - List of interconnected notes
- `sample_markdown_files` - Pre-created markdown files
- `create_sample_notes` - Create notes in vault

### Security Fixtures
- `path_traversal_attempts` - Common attack patterns
- `malicious_wikilink_patterns` - Wikilink edge cases

### OAuth Fixtures
- `oauth_state_token` - CSRF state parameter
- `oauth_code` - Authorization code
- `valid_access_token` - Access token
- `expired_token_data` - Expired token for testing

## Test Data

Sample test data is available in `tests/fixtures/`:

- `sample_notes.py` - Functions to generate test notes
- `test_vault/*.md` - Sample markdown files

## Coverage Report

After running tests with coverage:

```bash
pytest --cov=app --cov-report=html
```

Open `htmlcov/index.html` in your browser to view the detailed coverage report.

## Continuous Integration

Tests are designed to run in CI/CD pipelines. The sentence-transformers model is cached to avoid repeated downloads.

### GitHub Actions (To Be Implemented)

```yaml
- name: Cache sentence-transformers
  uses: actions/cache@v3
  with:
    path: ~/.cache/torch
    key: sentence-transformers-all-MiniLM-L6-v2
```

## Writing New Tests

### Test Structure Template

```python
import pytest
from app.services.your_service import YourService

@pytest.mark.unit
class TestYourFeature:
    """Test description"""

    def test_basic_functionality(self, your_fixture):
        """Test description"""
        # Arrange
        input_data = "test"

        # Act
        result = your_fixture.method(input_data)

        # Assert
        assert result == expected_value
```

### Security Tests

All security-critical tests should use the `@pytest.mark.security` marker:

```python
@pytest.mark.security
def test_path_traversal_protection(self, knowledge_store):
    """Test that path traversal is blocked"""
    with pytest.raises(ValueError):
        knowledge_store.get_note("../../etc/passwd")
```

## Troubleshooting

### Model Download Issues

If sentence-transformers model fails to download:

```bash
# Pre-download the model
python -c "from sentence_transformers import SentenceTransformer; SentenceTransformer('all-MiniLM-L6-v2')"
```

### Permission Errors on Windows

File permission tests are automatically skipped on Windows:

```python
@pytest.mark.skipif(os.name == 'nt', reason="Unix permissions not applicable on Windows")
```

### LanceDB Issues

If LanceDB tests fail:

```bash
# Ensure LanceDB is properly installed
pip install lancedb --upgrade
```

## Test Statistics

- **Total Tests Created**: 210+
- **Test Files**: 5
- **Coverage Target**: 80%+
- **Security Tests**: 40+
- **Performance Tests**: 10+

## Next Steps

To complete the test suite:

1. **Middleware Tests** - Create test_security.py, test_rate_limit.py, test_logging.py
2. **Model Tests** - Create test_note_models.py for Pydantic validation
3. **Integration Tests** - Create API endpoint integration tests
4. **Frontend Tests** - Set up Vitest and create component/store tests
5. **E2E Tests** - Set up Playwright and create workflow tests
6. **CI/CD** - Configure GitHub Actions workflow

## Contributing

When adding new tests:

1. Follow the existing test structure and patterns
2. Use appropriate pytest markers
3. Add docstrings to test classes and functions
4. Use fixtures from conftest.py
5. Test both success and failure cases
6. Include edge cases and security tests
7. Update this README with test counts

## References

- [pytest Documentation](https://docs.pytest.org/)
- [pytest-asyncio](https://pytest-asyncio.readthedocs.io/)
- [FastAPI Testing](https://fastapi.tiangolo.com/tutorial/testing/)
- [sentence-transformers](https://www.sbert.net/)
- [LanceDB Documentation](https://lancedb.github.io/lancedb/)

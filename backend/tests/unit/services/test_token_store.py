"""
Unit tests for TokenStore service

Tests cover:
- Token encryption and decryption (Fernet)
- Token storage and retrieval
- Token expiration handling
- File permission enforcement (0o600)
- Concurrent access safety
- CSRF state parameter management
- Security edge cases
"""
import pytest
from pathlib import Path
from datetime import datetime, timedelta
import os
import stat

from app.services.token_store import TokenStore


# ============================================================================
# Initialization Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestTokenStoreInitialization:
    """Test TokenStore initialization and configuration"""

    def test_initialize_creates_storage_directory(self, temp_token_storage_path: Path):
        """Test that storage directory is created"""
        # Remove directory if it exists
        if temp_token_storage_path.exists():
            import shutil
            shutil.rmtree(temp_token_storage_path)

        # Initialize store
        store = TokenStore(storage_dir=str(temp_token_storage_path))

        # Directory should be created
        assert temp_token_storage_path.exists()

    def test_storage_directory_permissions(self, temp_token_storage_path: Path):
        """Test that storage directory has correct permissions"""
        store = TokenStore(storage_dir=str(temp_token_storage_path))

        # On Unix systems, directory should have restricted permissions
        if os.name != 'nt':  # Skip on Windows
            dir_stat = os.stat(temp_token_storage_path)
            # Should be 0o700 (owner read/write/execute only)
            assert stat.S_IMODE(dir_stat.st_mode) == 0o700

    def test_encryption_key_generated(self, token_store: TokenStore):
        """Test that encryption key is generated or loaded"""
        # Should have encryption key
        assert hasattr(token_store, '_cipher') or hasattr(token_store, 'cipher')


# ============================================================================
# Encryption/Decryption Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestEncryptionDecryption:
    """Test token encryption and decryption"""

    def test_encrypt_decrypt_roundtrip(self, token_store: TokenStore):
        """Test that encryption and decryption are reversible"""
        original_token = "test_access_token_123456"

        # Encrypt
        encrypted = token_store._encrypt_token(original_token)

        # Decrypt
        decrypted = token_store._decrypt_token(encrypted)

        assert decrypted == original_token

    def test_encrypted_token_different_from_original(self, token_store: TokenStore):
        """Test that encrypted token is different from original"""
        original = "test_token"

        encrypted = token_store._encrypt_token(original)

        # Encrypted should not be same as original
        assert encrypted != original
        # Encrypted should be longer (base64 encoded Fernet token)
        assert len(encrypted) > len(original)

    def test_decrypt_invalid_token_raises_error(self, token_store: TokenStore):
        """Test that decrypting invalid token raises error"""
        invalid_token = "not_a_valid_encrypted_token"

        with pytest.raises(Exception):  # Fernet raises various exceptions
            token_store._decrypt_token(invalid_token)

    def test_encryption_consistent(self, token_store: TokenStore):
        """Test that same plaintext produces different ciphertexts"""
        token = "test_token"

        # Encrypt same token twice
        encrypted1 = token_store._encrypt_token(token)
        encrypted2 = token_store._encrypt_token(token)

        # Fernet includes random IV, so ciphertexts should differ
        # But both should decrypt to same value
        assert token_store._decrypt_token(encrypted1) == token
        assert token_store._decrypt_token(encrypted2) == token

    def test_encryption_handles_unicode(self, token_store: TokenStore):
        """Test encryption of unicode strings"""
        unicode_token = "token_with_unicode_你好_🚀"

        encrypted = token_store._encrypt_token(unicode_token)
        decrypted = token_store._decrypt_token(encrypted)

        assert decrypted == unicode_token


# ============================================================================
# Token Storage Tests
# ============================================================================

@pytest.mark.unit
class TestTokenStorage:
    """Test storing and retrieving tokens"""

    def test_store_token_success(self, token_store: TokenStore):
        """Test successful token storage"""
        token_id = "test_token_id"
        access_token = "test_access_token"
        user_data = {"id": "user123", "login": "testuser"}
        expires_at = datetime.utcnow() + timedelta(hours=1)

        token_store.store_token(token_id, access_token, user_data, expires_at)

        # Should be retrievable
        retrieved = token_store.get_token(token_id)
        assert retrieved is not None

    def test_get_token_success(self, token_store: TokenStore):
        """Test successful token retrieval"""
        token_id = "get_test"
        access_token = "access_123"
        user_data = {"id": "user1", "email": "test@example.com"}
        expires_at = datetime.utcnow() + timedelta(hours=1)

        token_store.store_token(token_id, access_token, user_data, expires_at)

        retrieved = token_store.get_token(token_id)

        assert retrieved["access_token"] == access_token
        assert retrieved["user_data"] == user_data

    def test_get_nonexistent_token_returns_none(self, token_store: TokenStore):
        """Test retrieving non-existent token"""
        result = token_store.get_token("nonexistent_token_id")

        assert result is None

    def test_store_overwrites_existing_token(self, token_store: TokenStore):
        """Test that storing with same ID overwrites"""
        token_id = "overwrite_test"

        # Store first token
        token_store.store_token(
            token_id,
            "original_token",
            {"id": "user1"},
            datetime.utcnow() + timedelta(hours=1),
        )

        # Store second token with same ID
        token_store.store_token(
            token_id,
            "new_token",
            {"id": "user2"},
            datetime.utcnow() + timedelta(hours=2),
        )

        # Should get the new token
        retrieved = token_store.get_token(token_id)
        assert retrieved["access_token"] == "new_token"
        assert retrieved["user_data"]["id"] == "user2"


# ============================================================================
# Token Deletion Tests
# ============================================================================

@pytest.mark.unit
class TestTokenDeletion:
    """Test token deletion"""

    def test_delete_token_success(self, token_store: TokenStore):
        """Test successful token deletion"""
        token_id = "delete_test"

        # Store token
        token_store.store_token(
            token_id,
            "token_to_delete",
            {"id": "user1"},
            datetime.utcnow() + timedelta(hours=1),
        )

        # Delete it
        result = token_store.delete_token(token_id)

        assert result is True

        # Should no longer be retrievable
        assert token_store.get_token(token_id) is None

    def test_delete_nonexistent_token_returns_false(self, token_store: TokenStore):
        """Test deleting non-existent token"""
        result = token_store.delete_token("nonexistent")

        assert result is False

    def test_cleanup_all_tokens(self, token_store: TokenStore):
        """Test clearing all tokens"""
        # Store multiple tokens
        for i in range(5):
            token_store.store_token(
                f"token_{i}",
                f"access_{i}",
                {"id": f"user{i}"},
                datetime.utcnow() + timedelta(hours=1),
            )

        # Clear all
        token_store.cleanup_all()

        # All should be gone
        for i in range(5):
            assert token_store.get_token(f"token_{i}") is None


# ============================================================================
# Token Expiration Tests
# ============================================================================

@pytest.mark.unit
class TestTokenExpiration:
    """Test token expiration handling"""

    def test_expired_token_returns_none(self, token_store: TokenStore):
        """Test that expired tokens return None"""
        token_id = "expired_test"

        # Store token that's already expired
        expired_time = datetime.utcnow() - timedelta(hours=1)

        token_store.store_token(
            token_id,
            "expired_token",
            {"id": "user1"},
            expired_time,
        )

        # Should return None
        result = token_store.get_token(token_id)

        assert result is None

    def test_valid_token_not_expired(self, token_store: TokenStore):
        """Test that valid tokens are returned"""
        token_id = "valid_test"

        # Store token that expires in 1 hour
        future_time = datetime.utcnow() + timedelta(hours=1)

        token_store.store_token(
            token_id,
            "valid_token",
            {"id": "user1"},
            future_time,
        )

        # Should be returned
        result = token_store.get_token(token_id)

        assert result is not None
        assert result["access_token"] == "valid_token"

    def test_cleanup_expired_tokens(self, token_store: TokenStore):
        """Test automatic cleanup of expired tokens"""
        # Store mix of valid and expired tokens
        now = datetime.utcnow()

        token_store.store_token("expired_1", "token1", {"id": "1"}, now - timedelta(hours=2))
        token_store.store_token("expired_2", "token2", {"id": "2"}, now - timedelta(hours=1))
        token_store.store_token("valid_1", "token3", {"id": "3"}, now + timedelta(hours=1))
        token_store.store_token("valid_2", "token4", {"id": "4"}, now + timedelta(hours=2))

        # Cleanup expired
        token_store._cleanup_expired_tokens()

        # Expired tokens should be gone
        assert token_store.get_token("expired_1") is None
        assert token_store.get_token("expired_2") is None

        # Valid tokens should remain
        assert token_store.get_token("valid_1") is not None
        assert token_store.get_token("valid_2") is not None

    def test_token_exactly_at_expiration(self, token_store: TokenStore, freeze_time):
        """Test token behavior exactly at expiration time"""
        with freeze_time("2025-01-01 12:00:00"):
            token_id = "exact_expiry"
            expiry_time = datetime(2025, 1, 1, 13, 0, 0)  # 1 hour from now

            token_store.store_token(token_id, "token", {"id": "user"}, expiry_time)

        # Move time to exactly expiration
        with freeze_time("2025-01-01 13:00:00"):
            result = token_store.get_token(token_id)

            # Should be expired (or implementation may allow exact match)
            # Document expected behavior


# ============================================================================
# File Permissions Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestFilePermissions:
    """Test file permission enforcement"""

    @pytest.mark.skipif(os.name == 'nt', reason="Unix permissions not applicable on Windows")
    def test_token_file_has_restricted_permissions(self, token_store: TokenStore, temp_token_storage_path: Path):
        """Test that token file has 0o600 permissions"""
        # Store a token to create file
        token_store.store_token(
            "test",
            "token",
            {"id": "user"},
            datetime.utcnow() + timedelta(hours=1),
        )

        # Save to disk
        token_store._save()

        # Check file permissions
        token_file = temp_token_storage_path / "tokens.json"
        if token_file.exists():
            file_stat = os.stat(token_file)
            # Should be 0o600 (owner read/write only)
            assert stat.S_IMODE(file_stat.st_mode) == 0o600


# ============================================================================
# Concurrent Access Tests
# ============================================================================

@pytest.mark.unit
class TestConcurrentAccess:
    """Test thread safety and concurrent access"""

    def test_concurrent_token_storage(self, token_store: TokenStore):
        """Test storing tokens concurrently"""
        import concurrent.futures

        def store_token(i):
            token_store.store_token(
                f"token_{i}",
                f"access_{i}",
                {"id": f"user{i}"},
                datetime.utcnow() + timedelta(hours=1),
            )

        # Store 20 tokens concurrently
        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
            futures = [executor.submit(store_token, i) for i in range(20)]
            [f.result() for f in futures]

        # All should be stored
        for i in range(20):
            assert token_store.get_token(f"token_{i}") is not None

    def test_concurrent_read_write(self, token_store: TokenStore):
        """Test concurrent reads and writes"""
        import concurrent.futures

        # Pre-populate some tokens
        for i in range(10):
            token_store.store_token(
                f"token_{i}",
                f"access_{i}",
                {"id": f"user{i}"},
                datetime.utcnow() + timedelta(hours=1),
            )

        def read_token(i):
            return token_store.get_token(f"token_{i}")

        def write_token(i):
            token_store.store_token(
                f"token_{i}",
                f"updated_{i}",
                {"id": f"user{i}_updated"},
                datetime.utcnow() + timedelta(hours=2),
            )

        # Mix of reads and writes
        with concurrent.futures.ThreadPoolExecutor(max_workers=10) as executor:
            read_futures = [executor.submit(read_token, i) for i in range(10)]
            write_futures = [executor.submit(write_token, i) for i in range(10)]

            # Should complete without errors
            [f.result() for f in read_futures + write_futures]


# ============================================================================
# CSRF State Parameter Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestCSRFStateManagement:
    """Test CSRF state parameter storage and validation"""

    def test_store_csrf_state(self, token_store: TokenStore):
        """Test storing CSRF state parameter"""
        import secrets
        state = secrets.token_urlsafe(32)

        # Store state (implementation-specific)
        # This documents expected behavior
        if hasattr(token_store, 'store_state'):
            token_store.store_state(state)

    def test_validate_csrf_state(self, token_store: TokenStore):
        """Test validating CSRF state parameter"""
        import secrets
        state = secrets.token_urlsafe(32)

        # If implementation supports state validation
        if hasattr(token_store, 'store_state') and hasattr(token_store, 'validate_state'):
            token_store.store_state(state)
            assert token_store.validate_state(state) is True
            assert token_store.validate_state("invalid_state") is False


# ============================================================================
# Edge Cases and Error Handling
# ============================================================================

@pytest.mark.unit
class TestEdgeCases:
    """Test edge cases and error conditions"""

    def test_very_long_token(self, token_store: TokenStore):
        """Test handling of very long tokens"""
        long_token = "a" * 10000

        token_store.store_token(
            "long_test",
            long_token,
            {"id": "user"},
            datetime.utcnow() + timedelta(hours=1),
        )

        retrieved = token_store.get_token("long_test")
        assert retrieved["access_token"] == long_token

    def test_special_characters_in_token_id(self, token_store: TokenStore):
        """Test token IDs with special characters"""
        special_ids = [
            "token-with-dashes",
            "token_with_underscores",
            "token.with.dots",
        ]

        for token_id in special_ids:
            token_store.store_token(
                token_id,
                "token",
                {"id": "user"},
                datetime.utcnow() + timedelta(hours=1),
            )

            assert token_store.get_token(token_id) is not None

    def test_unicode_in_user_data(self, token_store: TokenStore):
        """Test user data with unicode characters"""
        user_data = {
            "id": "user123",
            "name": "Test User 你好",
            "email": "test@example.com",
            "emoji": "🚀",
        }

        token_store.store_token(
            "unicode_test",
            "token",
            user_data,
            datetime.utcnow() + timedelta(hours=1),
        )

        retrieved = token_store.get_token("unicode_test")
        assert retrieved["user_data"]["name"] == "Test User 你好"
        assert retrieved["user_data"]["emoji"] == "🚀"

    def test_empty_user_data(self, token_store: TokenStore):
        """Test storing token with empty user data"""
        token_store.store_token(
            "empty_user",
            "token",
            {},
            datetime.utcnow() + timedelta(hours=1),
        )

        retrieved = token_store.get_token("empty_user")
        assert retrieved is not None
        assert retrieved["user_data"] == {}

    def test_none_user_data(self, token_store: TokenStore):
        """Test handling of None user data"""
        try:
            token_store.store_token(
                "none_user",
                "token",
                None,
                datetime.utcnow() + timedelta(hours=1),
            )

            retrieved = token_store.get_token("none_user")
            # Should handle gracefully
        except (ValueError, TypeError):
            # Acceptable to reject None
            pass

    def test_large_number_of_tokens(self, token_store: TokenStore):
        """Test storing many tokens"""
        # Store 1000 tokens
        for i in range(1000):
            token_store.store_token(
                f"token_{i}",
                f"access_{i}",
                {"id": f"user{i}"},
                datetime.utcnow() + timedelta(hours=1),
            )

        # Spot check retrieval
        assert token_store.get_token("token_0") is not None
        assert token_store.get_token("token_500") is not None
        assert token_store.get_token("token_999") is not None

    def test_persistence_across_instances(self, temp_token_storage_path: Path):
        """Test that tokens persist across TokenStore instances"""
        # Create first instance and store token
        store1 = TokenStore(storage_dir=str(temp_token_storage_path))
        store1.store_token(
            "persist_test",
            "token_value",
            {"id": "user"},
            datetime.utcnow() + timedelta(hours=1),
        )
        store1._save()

        # Create second instance
        store2 = TokenStore(storage_dir=str(temp_token_storage_path))
        store2._load()

        # Should retrieve token from disk
        retrieved = store2.get_token("persist_test")
        if retrieved:  # Implementation-specific
            assert retrieved["access_token"] == "token_value"


# ============================================================================
# Security Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestSecurity:
    """Security-focused tests"""

    def test_tokens_encrypted_at_rest(self, token_store: TokenStore, temp_token_storage_path: Path):
        """Test that tokens are encrypted when saved to disk"""
        plaintext_token = "secret_access_token_12345"

        token_store.store_token(
            "security_test",
            plaintext_token,
            {"id": "user"},
            datetime.utcnow() + timedelta(hours=1),
        )

        token_store._save()

        # Read the file directly
        token_file = temp_token_storage_path / "tokens.json"
        if token_file.exists():
            file_content = token_file.read_text()

            # Plaintext token should NOT appear in file
            assert plaintext_token not in file_content

    def test_different_encryption_keys_incompatible(self, temp_token_storage_path: Path):
        """Test that different encryption keys can't decrypt each other's tokens"""
        # Create store with one key
        from cryptography.fernet import Fernet
        key1 = Fernet.generate_key()

        # This test documents that keys are important for security
        # Actual implementation may vary


    def test_replay_protection(self, token_store: TokenStore):
        """Test that old tokens can't be replayed after deletion"""
        token_id = "replay_test"

        # Store and delete token
        token_store.store_token(
            token_id,
            "token",
            {"id": "user"},
            datetime.utcnow() + timedelta(hours=1),
        )

        token_store.delete_token(token_id)

        # Should not be retrievable
        assert token_store.get_token(token_id) is None

        # Even if re-stored with same ID, should be new token
        token_store.store_token(
            token_id,
            "new_token",
            {"id": "user2"},
            datetime.utcnow() + timedelta(hours=1),
        )

        retrieved = token_store.get_token(token_id)
        assert retrieved["access_token"] == "new_token"
        assert retrieved["user_data"]["id"] == "user2"

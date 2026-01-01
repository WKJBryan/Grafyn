"""Token storage service for OAuth tokens with encryption"""
import json
from pathlib import Path
from typing import Optional, Dict, Any
from datetime import datetime, timezone
import logging
import os
import secrets
from cryptography.fernet import Fernet
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC
import base64

logger = logging.getLogger(__name__)


class TokenStore:
    """Secure token storage with encryption and expiration support"""
    
    def __init__(self, storage_path: str = "data/tokens.json"):
        """Initialize token store with encryption"""
        self.storage_path = Path(storage_path)
        self.storage_path.parent.mkdir(parents=True, exist_ok=True)
        self._tokens: Dict[str, Any] = {}
        self._cipher = None
        
        # Set restrictive file permissions on Unix-like systems
        try:
            os.chmod(self.storage_path.parent, 0o700)
        except (OSError, AttributeError):
            logger.warning("Could not set restrictive permissions on token storage directory")
        
        self._load()
        self._initialize_encryption()
    
    def _initialize_encryption(self):
        """Initialize encryption using environment variable or generate key"""
        encryption_key = os.environ.get('TOKEN_ENCRYPTION_KEY')
        
        if encryption_key:
            # Use provided encryption key
            try:
                key_bytes = base64.urlsafe_b64decode(encryption_key.encode())
                self._cipher = Fernet(key_bytes)
            except Exception as e:
                logger.error(f"Failed to initialize encryption with provided key: {e}")
                self._generate_encryption_key()
        else:
            self._generate_encryption_key()
    
    def _generate_encryption_key(self):
        """Generate and store a new encryption key"""
        key_file = self.storage_path.parent / ".encryption_key"
        
        if key_file.exists():
            try:
                with open(key_file, 'rb') as f:
                    key = f.read()
                self._cipher = Fernet(key)
                logger.info("Loaded existing encryption key")
                return
            except Exception as e:
                logger.error(f"Failed to load encryption key: {e}")
        
        # Generate new key
        key = Fernet.generate_key()
        self._cipher = Fernet(key)
        
        # Store key securely
        try:
            with open(key_file, 'wb') as f:
                f.write(key)
            os.chmod(key_file, 0o600)  # Restrictive permissions
            logger.warning("Generated new encryption key. Save TOKEN_ENCRYPTION_KEY to environment variable for persistence.")
        except Exception as e:
            logger.error(f"Failed to save encryption key: {e}")
    
    def _encrypt_token(self, token: str) -> str:
        """Encrypt a token"""
        if not self._cipher:
            return token
        try:
            return self._cipher.encrypt(token.encode()).decode()
        except Exception as e:
            logger.error(f"Token encryption failed: {e}")
            return token
    
    def _decrypt_token(self, encrypted_token: str) -> str:
        """Decrypt a token"""
        if not self._cipher:
            return encrypted_token
        try:
            return self._cipher.decrypt(encrypted_token.encode()).decode()
        except Exception as e:
            logger.error(f"Token decryption failed: {e}")
            return encrypted_token
    
    def _load(self):
        """Load tokens from file"""
        if self.storage_path.exists():
            try:
                with open(self.storage_path, 'r') as f:
                    self._tokens = json.load(f)
                
                # Clean up expired tokens on load
                self._cleanup_expired_tokens()
                
                # Set restrictive file permissions
                try:
                    os.chmod(self.storage_path, 0o600)
                except (OSError, AttributeError):
                    pass
                    
            except Exception as e:
                logger.error(f"Failed to load tokens: {e}")
                self._tokens = {}
    
    def _save(self):
        """Save tokens to file"""
        try:
            with open(self.storage_path, 'w') as f:
                json.dump(self._tokens, f, indent=2, default=str)
            
            # Ensure restrictive permissions
            try:
                os.chmod(self.storage_path, 0o600)
            except (OSError, AttributeError):
                pass
                
        except Exception as e:
            logger.error(f"Failed to save tokens: {e}")
    
    def _cleanup_expired_tokens(self):
        """Remove expired tokens from storage"""
        current_time = datetime.now(timezone.utc)
        expired_tokens = []
        
        for token_id, token_data in self._tokens.items():
            if isinstance(token_data, dict) and 'expires_at' in token_data:
                expires_at = datetime.fromisoformat(token_data['expires_at'])
                if current_time > expires_at:
                    expired_tokens.append(token_id)
        
        for token_id in expired_tokens:
            del self._tokens[token_id]
            logger.info(f"Removed expired token: {token_id[:10]}...")
        
        if expired_tokens:
            self._save()
    
    def store_token(self, token_id: str, access_token: str, expires_at: Optional[datetime] = None) -> None:
        """Store a token with optional expiration"""
        # Encrypt the access token
        encrypted_token = self._encrypt_token(access_token)
        
        token_data = {
            'token': encrypted_token,
            'created_at': datetime.now(timezone.utc).isoformat()
        }
        
        if expires_at:
            token_data['expires_at'] = expires_at.isoformat()
        
        self._tokens[token_id] = token_data
        self._save()
        logger.info(f"Stored token: {token_id[:10]}...")
    
    def get_token(self, token_id: str) -> Optional[Dict[str, Any]]:
        """Retrieve a token with metadata"""
        token_data = self._tokens.get(token_id)
        
        if not token_data:
            return None
        
        # Check expiration
        if isinstance(token_data, dict) and 'expires_at' in token_data:
            expires_at = datetime.fromisoformat(token_data['expires_at'])
            if datetime.now(timezone.utc) > expires_at:
                # Token expired, delete it
                self.delete_token(token_id)
                return None
        
        # Return token data with decrypted token
        result = token_data.copy()
        if 'token' in result:
            result['token'] = self._decrypt_token(result['token'])
        
        return result
    
    def delete_token(self, token_id: str) -> bool:
        """Delete a token"""
        if token_id in self._tokens:
            del self._tokens[token_id]
            self._save()
            logger.info(f"Deleted token: {token_id[:10]}...")
            return True
        return False
    
    def cleanup_all(self):
        """Remove all tokens from storage"""
        count = len(self._tokens)
        self._tokens.clear()
        self._save()
        logger.info(f"Cleared all {count} tokens")
        return count

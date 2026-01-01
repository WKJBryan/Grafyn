"""Configuration management for Seedream backend"""
from pydantic_settings import BaseSettings
from typing import List


class Settings(BaseSettings):
    """Application settings loaded from environment variables"""
    
    # Server Configuration
    server_host: str = "0.0.0.0"
    server_port: int = 8080
    
    # Paths
    vault_path: str = "../vault"
    data_path: str = "../data"
    
    # Embedding Model
    embedding_model: str = "all-MiniLM-L6-v2"
    
    # OAuth Configuration
    github_client_id: str = ""
    github_client_secret: str = ""
    github_redirect_uri: str = ""
    
    # CORS Configuration
    cors_origins: List[str] = ["http://localhost:5173", "http://localhost:3000"]
    
    # Environment
    environment: str = "development"
    
    class Config:
        env_file = ".env"
        env_file_encoding = "utf-8"
        case_sensitive = False


# Global settings instance
settings = Settings()


def get_settings() -> Settings:
    """Get the application settings instance"""
    return settings

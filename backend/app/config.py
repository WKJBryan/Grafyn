"""Configuration management for Grafyn backend"""

from pathlib import Path
from pydantic_settings import BaseSettings
from pydantic import field_validator
from typing import List, Optional


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

    # OpenRouter Configuration
    openrouter_api_key: str = ""
    app_url: str = "http://localhost:8080"
    canvas_data_path: str = "../data/canvas"

    # Import Configuration
    import_max_file_size: int = 1024  # MB
    import_temp_dir: str = "../data/import/temp"
    import_auto_distill: bool = True
    import_distillation_model: str = "anthropic/claude-3-haiku"
    import_default_depth: int = 2
    import_dedup_threshold: float = 0.85
    import_check_dupes: bool = True

    # Feedback Configuration (GitHub Issues)
    github_feedback_repo: str = ""  # Format: owner/repo-name
    github_feedback_token: str = ""  # PAT with issues:write scope

    # CORS Configuration
    cors_origins: Optional[str] = None

    @field_validator("cors_origins")
    @classmethod
    def parse_cors_origins(cls, v: Optional[str]) -> List[str]:
        if v is None:
            return ["http://localhost:5173", "http://localhost:3000"]
        return [origin.strip() for origin in v.split(",")]

    @field_validator("vault_path", "data_path")
    @classmethod
    def validate_paths(cls, v: str) -> str:
        path = Path(v).expanduser().resolve()
        if not path.exists():
            path.mkdir(parents=True, exist_ok=True)
        return str(path)

    # Rate Limiting
    rate_limit_enabled: bool = True
    rate_limit_per_day: int = 200
    rate_limit_per_hour: int = 50
    rate_limit_per_minute: int = 10

    # Environment
    environment: str = "development"

    class Config:
        env_file = ".env"
        env_file_encoding = "utf-8"
        case_sensitive = False
        extra = "ignore"


# Global settings instance
settings = Settings()


def get_settings() -> Settings:
    """Get the application settings instance"""
    return settings

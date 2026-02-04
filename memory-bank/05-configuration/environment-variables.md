# Environment Variables Reference

> **Purpose:** Document all environment variables for Grafyn configuration
> **Created:** 2025-12-31
> **Last Updated:** 2025-12-31
> **Status:** Active

## Overview

This document lists all environment variables used by Grafyn backend and frontend for configuration.

## Backend Environment Variables

### Required Variables

| Variable | Type | Default | Description |
|-----------|------|----------|-------------|
| `VAULT_PATH` | string | `../vault` | Path to Markdown notes directory |
| `DATA_PATH` | string | `../data` | Path to LanceDB vector storage |

### Optional Variables

| Variable | Type | Default | Description |
|-----------|------|----------|-------------|
| `SERVER_HOST` | string | `0.0.0.0` | Server bind address |
| `SERVER_PORT` | integer | `8080` | Server HTTP port |
| `EMBEDDING_MODEL` | string | `all-MiniLM-L6-v2` | Sentence transformer model name |
| `LOG_LEVEL` | string | `INFO` | Logging level (DEBUG, INFO, WARNING, ERROR) |
| `LOG_FILE` | string | `backend.log` | Log file path |
| `CORS_ORIGINS` | string | `*` | Allowed CORS origins (comma-separated) |
| `GITHUB_CLIENT_ID` | string | `None` | GitHub OAuth client ID (for ChatGPT) |
| `GITHUB_CLIENT_SECRET` | string | `None` | GitHub OAuth client secret (for ChatGPT) |
| `GITHUB_REDIRECT_URI` | string | `None` | OAuth callback URL (e.g., https://your-name.ngrok.io/auth/callback) |

### Configuration File

**File:** `backend/.env`

```bash
# Vault Configuration
VAULT_PATH=../vault
DATA_PATH=../data

# Server Configuration
SERVER_HOST=0.0.0.0
SERVER_PORT=8080

# Embedding Configuration
EMBEDDING_MODEL=all-MiniLM-L6-v2

# Logging Configuration
LOG_LEVEL=INFO
LOG_FILE=backend.log

# CORS Configuration (Production)
CORS_ORIGINS=https://yourdomain.com,https://app.yourdomain.com

# OAuth Configuration (for ChatGPT)
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback
```

### Example Configurations

#### Development

```bash
# Development configuration
VAULT_PATH=../vault
DATA_PATH=../data
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
EMBEDDING_MODEL=all-MiniLM-L6-v2
LOG_LEVEL=DEBUG
CORS_ORIGINS=*

# OAuth (optional for ChatGPT)
# GITHUB_CLIENT_ID=your-github-client-id
# GITHUB_CLIENT_SECRET=your-github-client-secret
# GITHUB_REDIRECT_URI=http://localhost:8080/auth/callback
```

#### Production

```bash
# Production configuration
VAULT_PATH=/var/lib/grafyn/vault
DATA_PATH=/var/lib/grafyn/data
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
EMBEDDING_MODEL=all-MiniLM-L6-v2
LOG_LEVEL=WARNING
LOG_FILE=/var/log/grafyn/backend.log
CORS_ORIGINS=https://app.yourdomain.com

# OAuth (required for ChatGPT)
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://api.yourdomain.com/auth/callback
```

#### Docker

```bash
# Docker configuration
VAULT_PATH=/data/vault
DATA_PATH=/data/lancedb
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
EMBEDDING_MODEL=all-MiniLM-L6-v2
LOG_LEVEL=INFO
LOG_FILE=/app/logs/backend.log
CORS_ORIGINS=*

# OAuth (for ChatGPT)
GITHUB_CLIENT_ID=${GITHUB_CLIENT_ID}
GITHUB_CLIENT_SECRET=${GITHUB_CLIENT_SECRET}
GITHUB_REDIRECT_URI=${GITHUB_REDIRECT_URI}
```

## Frontend Environment Variables

### Required Variables

None - frontend uses proxy configuration in vite.config.js

### Optional Variables

| Variable | Type | Default | Description |
|-----------|------|----------|-------------|
| `VITE_API_BASE_URL` | string | `/api` | Base URL for API calls |
| `VITE_SSE_BASE_URL` | string | `/sse` | Base URL for SSE/MCP endpoint |

### Configuration File

**File:** `frontend/.env` (optional)

```bash
# API Configuration
VITE_API_BASE_URL=/api
VITE_SSE_BASE_URL=/sse
```

### Example Configurations

#### Development

```bash
# Development configuration (uses Vite proxy)
VITE_API_BASE_URL=/api
VITE_SSE_BASE_URL=/sse
```

#### Production

```bash
# Production configuration (direct backend URL)
VITE_API_BASE_URL=https://api.yourdomain.com
VITE_SSE_BASE_URL=https://api.yourdomain.com/sse
```

## Environment Variable Validation

### Backend Validation

```python
# backend/app/config.py
from pydantic_settings import BaseSettings
from pydantic import validator

class Settings(BaseSettings):
    vault_path: str = "../vault"
    data_path: str = "../data"
    server_host: str = "0.0.0.0"
    server_port: int = 8080
    embedding_model: str = "all-MiniLM-L6-v2"
    log_level: str = "INFO"
    log_file: str = "backend.log"
    cors_origins: str = "*"
    github_client_id: str = None
    github_client_secret: str = None
    github_redirect_uri: str = None
    
    @validator('server_port')
    def validate_port(cls, v):
        if v < 1024 or v > 65535:
            raise ValueError('Port must be between 1024 and 65535')
        return v
    
    @validator('log_level')
    def validate_log_level(cls, v):
        valid_levels = ['DEBUG', 'INFO', 'WARNING', 'ERROR']
        if v not in valid_levels:
            raise ValueError(f'Log level must be one of: {valid_levels}')
        return v
    
    class Config:
        env_file = ".env"
```

### Frontend Validation

```javascript
// frontend/src/config.js
const validateConfig = () => {
  const required = []
  
  if (!import.meta.env.VITE_API_BASE_URL) {
    required.push('VITE_API_BASE_URL')
  }
  
  if (required.length > 0) {
    throw new Error(`Missing required environment variables: ${required.join(', ')}`)
  }
}

validateConfig()

export const config = {
  apiBaseUrl: import.meta.env.VITE_API_BASE_URL,
  sseBaseUrl: import.meta.env.VITE_SSE_BASE_URL
}
```

## Security Considerations

### Sensitive Variables

Never commit sensitive environment variables to version control:

```bash
# .gitignore
.env
.env.local
.env.production
*.key
*.pem

# OAuth secrets
GITHUB_CLIENT_SECRET
```

### Secrets Management

For production, use secrets management:

```bash
# Using environment variables
export DATABASE_PASSWORD="your-password"

# Using secrets manager (AWS)
export DATABASE_PASSWORD=$(aws secretsmanager get-secret-value --secret-id grafyn-db-password)

# Using Docker secrets
docker run -e DATABASE_PASSWORD_FILE=/run/secrets/db_password grafyn

# Using secrets for OAuth (recommended for production)
export GITHUB_CLIENT_SECRET=$(aws secretsmanager get-secret-value --secret-id grafyn-github-client-secret)
```

## Troubleshooting

### Issue: Environment Variables Not Loading

**Symptom:**
Configuration uses default values instead of .env values.

**Cause:**
- .env file not in correct location
- python-dotenv not installed
- .env file has syntax errors

**Solution:**
```bash
# Check .env exists
ls -la backend/.env

# Verify .env syntax
cat backend/.env

# Check python-dotenv is installed
pip list | grep dotenv

# Test loading
python -c "from dotenv import load_dotenv; load_dotenv(); import os; print(os.getenv('VAULT_PATH'))"
```

### Issue: Port Already in Use

**Symptom:**
```
OSError: [Errno 48] Address already in use
```

**Solution:**
```bash
# Use different port
export SERVER_PORT=8081

# Or kill existing process
lsof -ti:8080 | xargs kill -9
```

### Issue: Invalid Log Level

**Symptom:**
```
ValueError: Log level must be one of: ['DEBUG', 'INFO', 'WARNING', 'ERROR']
```

**Solution:**
```bash
# Use valid log level
export LOG_LEVEL=INFO  # Valid: DEBUG, INFO, WARNING, ERROR
```

### Issue: OAuth Configuration Missing

**Symptom:**
ChatGPT cannot connect to MCP server.

**Cause:**
- GitHub OAuth credentials not configured
- Redirect URI doesn't match GitHub OAuth app

**Solution:**
```bash
# Verify OAuth variables are set
echo $GITHUB_CLIENT_ID
echo $GITHUB_CLIENT_SECRET
echo $GITHUB_REDIRECT_URI

# Ensure redirect URI matches GitHub OAuth app exactly
# GitHub OAuth app must have: https://your-name.ngrok.io/auth/callback
```

## Best Practices

### 1. Use .env.example

Create `.env.example` file for reference:

```bash
# backend/.env.example
VAULT_PATH=../vault
DATA_PATH=../data
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
EMBEDDING_MODEL=all-MiniLM-L6-v2
LOG_LEVEL=INFO
LOG_FILE=backend.log
CORS_ORIGINS=*

# OAuth Configuration (for ChatGPT)
# GITHUB_CLIENT_ID=your-github-client-id
# GITHUB_CLIENT_SECRET=your-github-client-secret
# GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback
```

### 2. Document All Variables

Document all environment variables in README and this file.

### 3. Use Sensible Defaults

Provide defaults that work for most use cases.

### 4. Validate on Startup

Validate environment variables on application startup.

### 5. Use Environment-Specific Files

Use `.env.development`, `.env.production` for different environments.

### 6. Never Commit Secrets

Add `.env` to `.gitignore`:

```gitignore
# Environment variables
.env
.env.local
.env.*.local
```

### 7. Secure OAuth Secrets

- Never commit `GITHUB_CLIENT_SECRET` to version control
- Use environment variables or secrets manager in production
- Rotate OAuth secrets regularly
- Limit OAuth app permissions to minimum required scope

## Related Documentation

- [Setup Guide](./setup-guide.md)
- [Troubleshooting](./troubleshooting.md)
- [Development Guide - Backend](../../docs/development-guide-backend.md)
- [ADR-003: MCP Integration](../02-architecture-decisions/adr-003-mcp-integration.md)
- [ADR-006: OAuth Authentication](../02-architecture-decisions/adr-006-oauth-authentication.md)

---

**See Also:**
- [Architecture - Backend](../../docs/architecture-backend.md)
- [Configuration](../05-configuration/)
- [Development Patterns](../03-development-patterns/)

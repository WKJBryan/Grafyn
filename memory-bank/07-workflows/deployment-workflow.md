# Deployment Workflow

> **Purpose:** Guide for deploying OrgAI to production
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document defines the workflow for deploying OrgAI to production environments.

## Deployment Types

| Type | Description | Use Case |
|-------|-------------|----------|
| **Development** | Local development setup | Daily development |
| **Staging** | Pre-production testing | Testing before production |
| **Production** | Live deployment | End users |

## Prerequisites

### Before Deployment

- [ ] All tests pass
- [ ] Code reviewed and approved
- [ ] Documentation updated
- [ ] Version number updated
- [ ] CHANGELOG updated
- [ ] Security audit completed
- [ ] Performance tested
- [ ] Backup plan in place

### Environment Preparation

```bash
# Production server requirements
# - Python 3.10+
# - Node.js 18+
# - 2GB+ RAM
# - 10GB+ disk space
# - SSL certificate (optional)
```

## Development Deployment

### Local Development Setup

```bash
# Backend
cd backend
python -m venv venv
source venv/bin/activate  # Linux/Mac
venv\Scripts\activate  # Windows
pip install -r requirements.txt
pip install -r requirements-dev.txt
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080

# Frontend
cd frontend
npm install
npm run dev
```

### Access Points

| Service | URL |
|----------|-----|
| Frontend UI | http://localhost:5173 |
| Backend API | http://localhost:8080 |
| API Docs | http://localhost:8080/docs |
| MCP Endpoint | http://localhost:8080/mcp |

## Staging Deployment

### Preparation

```bash
# Create staging branch
git checkout main
git pull origin main
git checkout -b staging/deploy-<date>

# Update configuration
# Set staging environment variables
export VAULT_PATH=/var/lib/orgai/staging/vault
export DATA_PATH=/var/lib/orgai/staging/data
export SERVER_HOST=0.0.0.0
export SERVER_PORT=8080
export LOG_LEVEL=INFO
export CORS_ORIGINS=https://staging.yourdomain.com
```

### Deployment Steps

#### Option 1: Direct Deployment

```bash
# SSH to staging server
ssh user@staging-server

# Clone repository
git clone https://github.com/your-org/orgai.git
cd orgai
git checkout staging/deploy-<date>

# Setup Python environment
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

# Create directories
mkdir -p /var/lib/orgai/staging/vault
mkdir -p /var/lib/orgai/staging/data

# Configure environment
cat > .env << EOF
VAULT_PATH=/var/lib/orgai/staging/vault
DATA_PATH=/var/lib/orgai/staging/data
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
LOG_LEVEL=INFO
CORS_ORIGINS=https://staging.yourdomain.com
EOF

# Start backend (with supervisor)
uvicorn app.main:app --host 0.0.0.0 --port 8080 --workers 4

# Setup frontend
npm install
npm run build

# Serve with nginx
sudo cp -r dist/* /var/www/orgai-staging/
```

#### Option 2: Docker Deployment

```bash
# Build Docker images
docker build -t orgai-backend:latest ./backend
docker build -t orgai-frontend:latest ./frontend

# Run with Docker Compose
docker-compose -f docker-compose.staging.yml up -d

# docker-compose.staging.yml
version: '3.8'
services:
  backend:
    image: orgai-backend:latest
    ports:
      - "8080:8080"
    volumes:
      - /var/lib/orgai/staging/vault:/data/vault
      - /var/lib/orgai/staging/data:/data/lancedb
    environment:
      - VAULT_PATH=/data/vault
      - DATA_PATH=/data/lancedb
      - LOG_LEVEL=INFO

  frontend:
    image: orgai-frontend:latest
    ports:
      - "80:80"
    volumes:
      - ./nginx-staging.conf:/etc/nginx/nginx.conf
```

### Verification

```bash
# Health check
curl https://staging.yourdomain.com/health

# Test API
curl https://staging.yourdomain.com/api/notes

# Test frontend
# Open browser to: https://staging.yourdomain.com

# Check logs
ssh user@staging-server 'tail -f /var/log/orgai/backend.log'
```

## Production Deployment

### Preparation

```bash
# Create release branch
git checkout main
git pull origin main

# Update version
# Update version in backend/app/main.py
# Update version in frontend/package.json

# Create release tag
git tag -a v0.2.0 -m "Release 0.2.0"

# Push tag
git push origin main --tags
```

### Deployment Steps

#### Option 1: Systemd Service

```bash
# Create systemd service file
sudo cat > /etc/systemd/system/orgai-backend.service << EOF
[Unit]
Description=OrgAI Backend Service
After=network.target

[Service]
Type=simple
User=orgai
WorkingDirectory=/opt/orgai
Environment="PATH=/opt/orgai/venv/bin"
ExecStart=/opt/orgai/venv/bin/uvicorn app.main:app --host 0.0.0.0 --port 8080 --workers 4
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

# Enable and start service
sudo systemctl daemon-reload
sudo systemctl enable orgai-backend
sudo systemctl start orgai-backend

# Check status
sudo systemctl status orgai-backend
```

#### Option 2: Docker Deployment

```bash
# Build production images
docker build -t orgai-backend:v0.2.0 ./backend
docker build -t orgai-frontend:v0.2.0 ./frontend

# Tag and push
docker tag orgai-backend:v0.2.0 your-registry/orgai-backend:latest
docker tag orgai-frontend:v0.2.0 your-registry/orgai-frontend:latest
docker push your-registry/orgai-backend:latest
docker push your-registry/orgai-frontend:latest

# Deploy with Docker Compose
docker-compose -f docker-compose.prod.yml up -d

# docker-compose.prod.yml
version: '3.8'
services:
  backend:
    image: your-registry/orgai-backend:latest
    ports:
      - "8080:8080"
    volumes:
      - orgai-vault:/data/vault
      - orgai-data:/data/lancedb
      - orgai-logs:/var/log/orgai
    environment:
      - VAULT_PATH=/data/vault
      - DATA_PATH=/data/lancedb
      - LOG_LEVEL=WARNING
      - CORS_ORIGINS=https://app.yourdomain.com
    restart: always

  frontend:
    image: your-registry/orgai-frontend:latest
    ports:
      - "80:80"
    volumes:
      - ./nginx-prod.conf:/etc/nginx/nginx.conf
    restart: always

  nginx:
    image: nginx:alpine
    ports:
      - "443:443"
      - "80:80"
    volumes:
      - ./nginx-prod.conf:/etc/nginx/nginx.conf
      - ./ssl:/etc/nginx/ssl
    restart: always
```

#### Option 3: Cloud Deployment

```bash
# Deploy to cloud platform

# Heroku
heroku create orgai-prod
heroku addons:create heroku-postgresql
git push heroku main

# AWS ECS
# Use AWS CLI or Console
aws ecs create-cluster --cluster-name orgai-prod
aws ecs register-task-definition --family orgai
aws ecs run-task --cluster orgai-prod --task-definition orgai

# DigitalOcean
doctl apps create orgai-prod --region nyc3
doctl apps deploy orgai-prod --image your-registry/orgai:latest
```

### SSL/TLS Configuration

```nginx
# nginx configuration
server {
    listen 443 ssl http2;
    server_name app.yourdomain.com;
    
    ssl_certificate /etc/nginx/ssl/cert.pem;
    ssl_certificate_key /etc/nginx/ssl/key.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    
    location /api/ {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
    
    location /mcp/ {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
    
    location / {
        root /var/www/orgai;
        try_files $uri $uri/ /index.html;
    }
}
```

## Post-Deployment Verification

### Health Checks

```bash
# Backend health
curl https://app.yourdomain.com/health

# Expected response:
{
  "status": "healthy",
  "service": "orgai"
}

# API endpoint
curl https://app.yourdomain.com/api/notes

# Expected: Array of notes or empty array

# Frontend
curl -I https://app.yourdomain.com/

# Expected: 200 OK
```

### Functional Testing

```bash
# Create test note
curl -X POST https://app.yourdomain.com/api/notes \
  -H "Content-Type: application/json" \
  -d '{"title":"Deployment Test","content":"Testing deployment"}'

# Search for note
curl "https://app.yourdomain.com/api/search?q=Deployment%20Test"

# Verify in UI
# Open browser and test full workflow
```

### Performance Testing

```bash
# Load test with Apache Bench
ab -n 1000 -c 10 https://app.yourdomain.com/api/notes

# Expected: < 500ms average response time

# Monitor resources
top  # Check CPU/memory usage
df -h  # Check disk usage
```

## Monitoring

### Application Monitoring

```bash
# Backend logs
ssh user@prod-server 'tail -f /var/log/orgai/backend.log'

# Systemd service status
sudo systemctl status orgai-backend

# Docker logs
docker logs -f orgai-backend
docker logs -f orgai-frontend
```

### Metrics Collection

```python
# Add to backend/app/main.py
from prometheus_client import Counter, Histogram

note_creates = Counter('orgai_note_creates_total')
search_requests = Histogram('orgai_search_duration_seconds')

@app.post("/api/notes")
async def create_note(data: NoteCreate):
    note_creates.inc()
    # ... rest of code
```

### Alerting

```bash
# Set up alerts for:
# - High error rates
# - High response times
# - Service downtime
# - Disk space low
# - Memory usage high
```

## Rollback Procedure

### When to Rollback

- Critical bugs in production
- Performance degradation
- Security vulnerability discovered
- Data corruption

### Rollback Steps

```bash
# Stop current deployment
sudo systemctl stop orgai-backend
# Or
docker-compose down

# Restore previous version
git checkout v0.1.0

# Rebuild and redeploy
docker build -t orgai-backend:v0.1.0 ./backend
docker-compose up -d

# Verify rollback
curl https://app.yourdomain.com/health
```

## Backup Strategy

### Database Backups

```bash
# Backup LanceDB data
tar -czf orgai-data-$(date +%Y%m%d).tar.gz /var/lib/orgai/data

# Backup vault
tar -czf orgai-vault-$(date +%Y%m%d).tar.gz /var/lib/orgai/vault

# Upload to cloud storage
aws s3 cp orgai-data-$(date +%Y%m%d).tar.gz s3://backups/orgai/
```

### Automated Backups

```bash
# Cron job for daily backups
0 2 * * * /opt/scripts/backup-orgai.sh

# backup-orgai.sh
#!/bin/bash
DATE=$(date +%Y%m%d)
tar -czf /backups/orgai-data-$DATE.tar.gz /var/lib/orgai/data
tar -czf /backups/orgai-vault-$DATE.tar.gz /var/lib/orgai/vault
find /backups -mtime +30 -delete  # Keep 30 days
```

## Security Considerations

### Production Checklist

- [ ] CORS restricted to specific domains
- [ ] Rate limiting enabled
- [ ] SSL/TLS configured
- [ ] Firewall rules configured
- [ ] Regular security updates
- [ ] Input validation enabled
- [ ] Error messages don't leak info
- [ ] Logs don't contain sensitive data
- [ ] Secrets managed securely

### Security Hardening

```bash
# Update dependencies regularly
pip install --upgrade -r requirements.txt
npm audit fix

# Use environment variables for secrets
export DATABASE_PASSWORD=$(aws secretsmanager get-secret-value --secret-id orgai-db-password)

# Configure firewall
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw enable
```

## Maintenance

### Regular Maintenance Tasks

```bash
# Weekly
- Review logs for errors
- Check disk space
- Update dependencies
- Review security advisories

# Monthly
- Review performance metrics
- Optimize database
- Review backup strategy
- Update documentation

# Quarterly
- Security audit
- Performance review
- Capacity planning
- Disaster recovery test
```

## Troubleshooting Deployment

### Common Issues

#### Issue: Service Won't Start

```bash
# Check service status
sudo systemctl status orgai-backend

# Check logs
sudo journalctl -u orgai-backend -n 50

# Check port binding
sudo netstat -tulpn | grep :8080
```

#### Issue: High CPU Usage

```bash
# Identify process
top -p %CPU

# Check worker count
# Reduce workers if needed

# Profile application
python -m cProfile -s time app/main.py
```

#### Issue: Out of Memory

```bash
# Check memory usage
free -h

# Reduce worker count
# Optimize batch sizes
# Add memory limits to Docker
```

## Related Documentation

- [Environment Variables](../05-configuration/environment-variables.md)
- [Setup Guide](../05-configuration/setup-guide.md)
- [Troubleshooting](../05-configuration/troubleshooting.md)
- [Development Workflow](./development-workflow.md)

---

**See Also:**
- [Architecture - Backend](../../docs/architecture-backend.md)
- [Development Guide - Backend](../../docs/development-guide-backend.md)
- [IMPROVEMENTS.md](../../docs/IMPROVEMENTS.md)

# ChatGPT MCP Connection Setup Guide

This guide explains how to connect ChatGPT to your Seedream knowledge base using MCP (Model Context Protocol).

## Prerequisites

- Seedream backend running on port 8080
- ngrok account (free) for public tunneling
- GitHub account for OAuth

---

## Step 1: Start ngrok Tunnel

First, expose your local backend to the internet:

```bash
# Option A: Using pyngrok (Python)
cd C:\Users\bryan\Seedream
.venv\Scripts\python.exe -c "
from dotenv import load_dotenv
load_dotenv()
from pyngrok import ngrok, conf
conf.get_default().auth_token = 'YOUR_NGROK_TOKEN'
tunnel = ngrok.connect(8080)
print(f'Tunnel URL: {tunnel.public_url}')
input('Press Enter to stop...')
"
```

```bash
# Option B: Using ngrok CLI
ngrok http 8080
```

**Note the public URL** (e.g., `https://abc123.ngrok-free.dev`) - you'll need it for the next steps.

---

## Step 2: Create GitHub OAuth App

1. Go to: https://github.com/settings/developers
2. Click **"OAuth Apps"** → **"New OAuth App"**
3. Fill in the form:
   - **Application name**: `Seedream ChatGPT MCP`
   - **Homepage URL**: `https://your-ngrok-url.ngrok-free.dev`
   - **Authorization callback URL**: `https://your-ngrok-url.ngrok-free.dev/auth/callback`
4. Click **"Register application"**
5. Copy the **Client ID**
6. Click **"Generate a new client secret"** and copy it

---

## Step 3: Update .env Files

Update **both** `.env` files with your credentials:

### File 1: `C:\Users\bryan\Seedream\.env`

```bash
# OAuth Configuration (for ChatGPT MCP)
GITHUB_CLIENT_ID=Ov23lirLaMQW1MMI9rgT
GITHUB_CLIENT_SECRET=e17b878b75b505784acc0c115cc924ed0d0704c5
GITHUB_REDIRECT_URI=https://your-ngrok-url.ngrok-free.dev/auth/callback

# Security Configuration
TOKEN_ENCRYPTION_KEY=9p-LlCYNIDYXYCD6HfbnkdX8Z3JzpGVlpts1jB8EZOQ=

# ngrok Configuration
NGROK_AUTH_TOKEN=your-ngrok-authtoken
```

### File 2: `C:\Users\bryan\Seedream\backend\.env`

```bash
# Same values as above
GITHUB_CLIENT_ID=Ov23lirLaMQW1MMI9rgT
GITHUB_CLIENT_SECRET=e17b878b75b505784acc0c115cc924ed0d0704c5
GITHUB_REDIRECT_URI=https://your-ngrok-url.ngrok-free.dev/auth/callback
TOKEN_ENCRYPTION_KEY=9p-LlCYNIDYXYCD6HfbnkdX8Z3JzpGVlpts1jB8EZOQ=
NGROK_AUTH_TOKEN=your-ngrok-authtoken
```

---

## Step 4: Start the Backend

```bash
cd C:\Users\bryan\Seedream
.venv\Scripts\python.exe -m backend.app.main
```

You should see:
```
INFO:     Uvicorn running on http://0.0.0.0:8080
INFO:     Application startup complete.
```

---

## Step 5: Verify Setup

Test that everything is working:

```bash
# Test health endpoint
curl https://your-ngrok-url.ngrok-free.dev/health

# Test OAuth endpoint (should return your GitHub Client ID)
curl https://your-ngrok-url.ngrok-free.dev/api/oauth/authorize/github

# Test MCP SSE endpoint
curl https://your-ngrok-url.ngrok-free.dev/sse
```

Expected responses:
- Health: `{"status":"healthy","service":"seedream","environment":"development"}`
- OAuth: `{"authorization_url":"https://github.com/login/oauth/authorize?client_id=YOUR_CLIENT_ID...}`
- SSE: `event: endpoint\ndata: /sse/messages/?session_id=...`

---

## Step 6: Connect from ChatGPT

1. Open ChatGPT (https://chatgpt.com)
2. Go to **Settings** → **Connected Apps** or **MCP Servers**
3. Click **"Add MCP Server"**
4. Enter the URL: `https://your-ngrok-url.ngrok-free.dev/sse`
5. Click **"Connect"**

### OAuth Flow

1. ChatGPT will redirect you to GitHub
2. Click **"Authorize"** to allow the app access
3. You'll be redirected back to the callback URL
4. ChatGPT will receive an access token
5. Connection established!

---

## Step 7: Use MCP with ChatGPT

Once connected, you can ask ChatGPT to:

- "Search my knowledge base for [topic]"
- "What notes do I have about [subject]?"
- "Show me backlinks for [note title]"
- "Find similar notes to [note name]"
- "List all notes with tag #[tagname]"

---

## Troubleshooting

### "ERR_NGROK_3200" - Tunnel Offline

**Cause**: ngrok tunnel is not running

**Solution**: Restart the ngrok tunnel (Step 1)

### "GitHub OAuth not configured"

**Cause**: Backend not reading from correct .env file

**Solution**: Update **both** `.env` files (project root AND backend directory)

### "Invalid or expired OAuth token"

**Cause**: Token expired after 1 hour

**Solution**: Reconnect the MCP server in ChatGPT

### ngrok URL Changed After Restart

**Cause**: Free ngrok tier generates new URLs each session

**Solution**:
1. Copy the new ngrok URL
2. Update GitHub OAuth app callback URL
3. Update .env `GITHUB_REDIRECT_URI`
4. Restart backend

### Port 8080 Already in Use

**Solution**:
```bash
# Kill existing process
taskkill /F /IM python.exe

# Or find and kill specific process
netstat -ano | findstr ":8080"
taskkill /F /PID <PID>
```

---

## Current Configuration

Your current setup (if you followed this guide):

| Setting | Value |
|---------|-------|
| Public URL | `https://spirographic-sympathetically-thiago.ngrok-free.dev` |
| MCP Endpoint | `https://spirographic-sympathetically-thiago.ngrok-free.dev/sse` |
| GitHub Client ID | `Ov23lirLaMQW1MMI9rgT` |

---

## Quick Start (After Initial Setup)

For subsequent sessions, you only need to:

1. Start ngrok tunnel
2. Start backend
3. Add MCP server URL in ChatGPT (if not already saved)

The .env configuration persists between sessions.

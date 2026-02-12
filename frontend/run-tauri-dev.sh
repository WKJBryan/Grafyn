#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Start Python backend in background (skip if port 8080 already in use)
if ! lsof -i :8080 -sTCP:LISTEN >/dev/null 2>&1; then
    echo "Starting Python backend on port 8080..."
    (cd "$SCRIPT_DIR/../backend" && python -m uvicorn app.main:app --reload --host 0.0.0.0 --port 8080) &
    PYTHON_PID=$!
    sleep 3
else
    echo "Python backend already running on port 8080, skipping."
fi

npm run tauri:dev

# Cleanup: kill Python backend when Tauri exits
if [ -n "$PYTHON_PID" ]; then
    kill $PYTHON_PID 2>/dev/null
fi

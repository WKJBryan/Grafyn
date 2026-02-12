@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set PATH=%USERPROFILE%\.cargo\bin;%PATH%
cd /d "%~dp0"

REM Start Python backend in a separate window (skip if port 8080 already in use)
netstat -ano | findstr ":8080 " | findstr "LISTENING" >nul 2>&1
if errorlevel 1 (
    echo Starting Python backend on port 8080...
    start "Grafyn Python Backend" /d "%~dp0..\backend" cmd /c "python -m uvicorn app.main:app --reload --host 0.0.0.0 --port 8080"
    timeout /t 3 /nobreak >nul
) else (
    echo Python backend already running on port 8080, skipping.
)

npm run tauri:dev

# -*- mode: python ; coding: utf-8 -*-
"""
PyInstaller spec file for Seedream Python backend sidecar.

This bundles the FastAPI backend as a standalone executable that Tauri
can launch as a sidecar process for MCP (Model Context Protocol) support.

Build with:
    pyinstaller seedream.spec

Or use the build script:
    python build-exe.py
"""

import sys
from pathlib import Path

# Determine the platform-specific executable name
if sys.platform == 'win32':
    exe_name = 'seedream-backend'
elif sys.platform == 'darwin':
    exe_name = 'seedream-backend'
else:
    exe_name = 'seedream-backend'

# Get the absolute path to the backend directory
backend_dir = Path(SPECPATH).resolve()

a = Analysis(
    ['app/main.py'],
    pathex=[str(backend_dir)],
    binaries=[],
    datas=[
        # Include any data files needed at runtime
        # The embedding model will be downloaded on first run
    ],
    hiddenimports=[
        # FastAPI and dependencies
        'fastapi',
        'uvicorn',
        'uvicorn.logging',
        'uvicorn.protocols',
        'uvicorn.protocols.http',
        'uvicorn.protocols.http.auto',
        'uvicorn.protocols.http.h11_impl',
        'uvicorn.protocols.http.httptools_impl',
        'uvicorn.protocols.websockets',
        'uvicorn.protocols.websockets.auto',
        'uvicorn.protocols.websockets.websockets_impl',
        'uvicorn.protocols.websockets.wsproto_impl',
        'uvicorn.lifespan',
        'uvicorn.lifespan.on',
        'uvicorn.lifespan.off',
        'starlette',
        'starlette.routing',
        'starlette.middleware',
        'starlette.responses',
        'starlette.requests',

        # Pydantic
        'pydantic',
        'pydantic_settings',
        'pydantic_core',

        # Vector DB and ML
        'lancedb',
        'pyarrow',
        'sentence_transformers',
        'torch',
        'transformers',
        'huggingface_hub',
        'tokenizers',

        # MCP integration
        'fastapi_mcp',
        'sse_starlette',

        # HTTP and async
        'httpx',
        'httpcore',
        'anyio',
        'sniffio',
        'h11',
        'httptools',
        'websockets',
        'wsproto',

        # Security
        'cryptography',

        # Utilities
        'frontmatter',
        'yaml',
        'aiofiles',
        'filelock',
        'dotenv',
        'slowapi',
        'limits',

        # Standard library extras often needed
        'multiprocessing',
        'concurrent.futures',
        'asyncio',
        'email.mime.text',
        'email.mime.multipart',

        # App modules (ensure they're all included)
        'app',
        'app.config',
        'app.routers',
        'app.routers.notes',
        'app.routers.search',
        'app.routers.graph',
        'app.routers.oauth',
        'app.routers.canvas',
        'app.routers.distill',
        'app.routers.priority',
        'app.routers.feedback',
        'app.routers.conversation_import',
        'app.routers.mcp_write',
        'app.routers.zettelkasten',
        'app.services',
        'app.services.knowledge_store',
        'app.services.vector_search',
        'app.services.graph_index',
        'app.services.embedding',
        'app.services.openrouter',
        'app.services.canvas_store',
        'app.services.token_store',
        'app.services.distillation',
        'app.services.import_service',
        'app.services.link_discovery',
        'app.services.priority_scoring',
        'app.services.priority_settings',
        'app.services.feedback',
        'app.services.parsers',
        'app.middleware',
        'app.middleware.logging',
        'app.middleware.security',
        'app.middleware.rate_limit',
        'app.mcp',
        'app.mcp.server',
        'app.mcp.tools',
        'app.models',
        'app.utils',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        # Exclude unnecessary packages to reduce size
        'tkinter',
        'matplotlib',
        'PIL',
        'scipy',
        'pandas',
        'numpy.testing',
        'pytest',
    ],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=None,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=None)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.zipfiles,
    a.datas,
    [],
    name=exe_name,
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,  # Compress with UPX if available
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,  # Keep console for logging output that Tauri captures
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)

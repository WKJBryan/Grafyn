#!/usr/bin/env python3
"""
Build script for Grafyn Python backend sidecar.

This script:
1. Builds the Python backend as a standalone executable using PyInstaller
2. Copies the executable to the Tauri binaries folder with correct naming

Usage:
    python build-exe.py [--clean] [--no-copy]

Options:
    --clean     Clean build artifacts before building
    --no-copy   Don't copy to Tauri binaries folder (just build)
"""

import argparse
import platform
import shutil
import subprocess
import sys
from pathlib import Path


def get_target_triple() -> str:
    """
    Get the Rust target triple for the current platform.

    Tauri expects sidecar binaries to be named with the target triple suffix:
    - Windows: grafyn-backend-x86_64-pc-windows-msvc.exe
    - macOS Intel: grafyn-backend-x86_64-apple-darwin
    - macOS ARM: grafyn-backend-aarch64-apple-darwin
    - Linux: grafyn-backend-x86_64-unknown-linux-gnu
    """
    system = platform.system().lower()
    machine = platform.machine().lower()

    if system == 'windows':
        if machine in ('amd64', 'x86_64'):
            return 'x86_64-pc-windows-msvc'
        elif machine in ('arm64', 'aarch64'):
            return 'aarch64-pc-windows-msvc'
        else:
            return 'i686-pc-windows-msvc'

    elif system == 'darwin':
        if machine in ('arm64', 'aarch64'):
            return 'aarch64-apple-darwin'
        else:
            return 'x86_64-apple-darwin'

    else:  # Linux and others
        if machine in ('amd64', 'x86_64'):
            return 'x86_64-unknown-linux-gnu'
        elif machine in ('arm64', 'aarch64'):
            return 'aarch64-unknown-linux-gnu'
        else:
            return 'i686-unknown-linux-gnu'


def clean_build_artifacts(backend_dir: Path) -> None:
    """Remove previous build artifacts."""
    print("Cleaning build artifacts...")

    dirs_to_clean = ['build', 'dist', '__pycache__']
    for dir_name in dirs_to_clean:
        dir_path = backend_dir / dir_name
        if dir_path.exists():
            shutil.rmtree(dir_path)
            print(f"  Removed {dir_path}")

    # Clean .spec-generated files
    for spec_file in backend_dir.glob('*.spec.bak'):
        spec_file.unlink()


def build_executable(backend_dir: Path) -> Path:
    """Build the executable using PyInstaller."""
    print("\nBuilding executable with PyInstaller...")
    print("This may take several minutes on first build.\n")

    spec_file = backend_dir / 'grafyn.spec'

    if not spec_file.exists():
        print(f"Error: Spec file not found: {spec_file}")
        sys.exit(1)

    # Run PyInstaller
    result = subprocess.run(
        [sys.executable, '-m', 'PyInstaller', '--clean', str(spec_file)],
        cwd=backend_dir,
        capture_output=False,  # Show output in real-time
    )

    if result.returncode != 0:
        print("\nPyInstaller build failed!")
        sys.exit(1)

    # Find the built executable
    dist_dir = backend_dir / 'dist'

    if platform.system() == 'Windows':
        exe_path = dist_dir / 'grafyn-backend.exe'
    else:
        exe_path = dist_dir / 'grafyn-backend'

    if not exe_path.exists():
        print(f"\nError: Expected executable not found: {exe_path}")
        sys.exit(1)

    print(f"\nBuild successful: {exe_path}")
    print(f"Size: {exe_path.stat().st_size / (1024 * 1024):.1f} MB")

    return exe_path


def copy_to_tauri(exe_path: Path, backend_dir: Path) -> Path:
    """Copy the executable to the Tauri binaries folder with correct naming."""
    print("\nCopying to Tauri binaries folder...")

    # Tauri binaries folder
    tauri_binaries = backend_dir.parent / 'frontend' / 'src-tauri' / 'binaries'
    tauri_binaries.mkdir(parents=True, exist_ok=True)

    # Get target triple for naming
    target_triple = get_target_triple()

    # Build the target filename
    if platform.system() == 'Windows':
        target_name = f'grafyn-backend-{target_triple}.exe'
    else:
        target_name = f'grafyn-backend-{target_triple}'

    target_path = tauri_binaries / target_name

    # Copy the file
    shutil.copy2(exe_path, target_path)

    # On Unix, ensure executable permissions
    if platform.system() != 'Windows':
        target_path.chmod(0o755)

    print(f"Copied to: {target_path}")

    return target_path


def verify_dependencies() -> bool:
    """Verify that required dependencies are installed."""
    print("Verifying dependencies...")

    try:
        import PyInstaller
        print(f"  PyInstaller: {PyInstaller.__version__}")
    except ImportError:
        print("\nError: PyInstaller not installed.")
        print("Install with: pip install pyinstaller")
        return False

    # Check for key dependencies
    required = ['fastapi', 'uvicorn', 'lancedb', 'sentence_transformers']
    missing = []

    for pkg in required:
        try:
            __import__(pkg)
        except ImportError:
            missing.append(pkg)

    if missing:
        print(f"\nWarning: Missing packages: {', '.join(missing)}")
        print("Run: pip install -r requirements.txt")
        return False

    return True


def main():
    parser = argparse.ArgumentParser(
        description='Build Grafyn Python backend as standalone executable'
    )
    parser.add_argument(
        '--clean',
        action='store_true',
        help='Clean build artifacts before building'
    )
    parser.add_argument(
        '--no-copy',
        action='store_true',
        help="Don't copy to Tauri binaries folder"
    )
    args = parser.parse_args()

    # Get paths
    backend_dir = Path(__file__).parent.resolve()

    print("=" * 60)
    print("Grafyn Python Backend Build Script")
    print("=" * 60)
    print(f"\nPlatform: {platform.system()} {platform.machine()}")
    print(f"Target triple: {get_target_triple()}")
    print(f"Backend dir: {backend_dir}")

    # Verify dependencies
    if not verify_dependencies():
        sys.exit(1)

    # Clean if requested
    if args.clean:
        clean_build_artifacts(backend_dir)

    # Build
    exe_path = build_executable(backend_dir)

    # Copy to Tauri
    if not args.no_copy:
        target_path = copy_to_tauri(exe_path, backend_dir)

        print("\n" + "=" * 60)
        print("BUILD COMPLETE")
        print("=" * 60)
        print(f"\nExecutable: {exe_path}")
        print(f"Tauri binary: {target_path}")
        print(f"\nNext steps:")
        print("1. Run 'npm run tauri:build' from frontend/ to build the app")
        print("2. Or run 'npm run tauri:dev' to test in development mode")
    else:
        print("\n" + "=" * 60)
        print("BUILD COMPLETE (not copied to Tauri)")
        print("=" * 60)
        print(f"\nExecutable: {exe_path}")


if __name__ == '__main__':
    main()

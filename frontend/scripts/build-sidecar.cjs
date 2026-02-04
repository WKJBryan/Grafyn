#!/usr/bin/env node
/**
 * Cross-platform script to build the Python sidecar for Tauri.
 *
 * This script:
 * 1. Checks if PyInstaller is installed
 * 2. Runs the Python build script
 * 3. Verifies the output exists
 *
 * Usage: node scripts/build-sidecar.cjs
 */

const { execSync, spawnSync } = require('child_process');
const path = require('path');
const fs = require('fs');

// Paths
const frontendDir = path.dirname(__dirname);
const backendDir = path.join(frontendDir, '..', 'backend');
const buildScript = path.join(backendDir, 'build-exe.py');
const binariesDir = path.join(frontendDir, 'src-tauri', 'binaries');

// Get target triple for current platform
function getTargetTriple() {
    const platform = process.platform;
    const arch = process.arch;

    if (platform === 'win32') {
        return arch === 'arm64'
            ? 'aarch64-pc-windows-msvc'
            : 'x86_64-pc-windows-msvc';
    } else if (platform === 'darwin') {
        return arch === 'arm64'
            ? 'aarch64-apple-darwin'
            : 'x86_64-apple-darwin';
    } else {
        return arch === 'arm64'
            ? 'aarch64-unknown-linux-gnu'
            : 'x86_64-unknown-linux-gnu';
    }
}

// Get expected binary name
function getExpectedBinaryName() {
    const triple = getTargetTriple();
    const ext = process.platform === 'win32' ? '.exe' : '';
    return `grafyn-backend-${triple}${ext}`;
}

console.log('========================================');
console.log('Building Python Sidecar for Tauri');
console.log('========================================');
console.log(`Platform: ${process.platform} ${process.arch}`);
console.log(`Target: ${getTargetTriple()}`);
console.log(`Backend dir: ${backendDir}`);
console.log('');

// Check if build script exists
if (!fs.existsSync(buildScript)) {
    console.error(`Error: Build script not found: ${buildScript}`);
    process.exit(1);
}

// Check if Python is available
try {
    const pythonVersion = execSync('python --version', { encoding: 'utf-8' }).trim();
    console.log(`Python: ${pythonVersion}`);
} catch (e) {
    console.error('Error: Python not found. Please install Python 3.x');
    process.exit(1);
}

// Check if PyInstaller is installed
try {
    execSync('python -c "import PyInstaller"', { stdio: 'pipe' });
    console.log('PyInstaller: installed');
} catch (e) {
    console.log('PyInstaller not found, installing...');
    try {
        execSync('pip install pyinstaller', { stdio: 'inherit' });
    } catch (installErr) {
        console.error('Error: Failed to install PyInstaller');
        process.exit(1);
    }
}

console.log('');
console.log('Running Python build script...');
console.log('(This may take several minutes on first build)');
console.log('');

// Run the build script
const result = spawnSync('python', [buildScript], {
    cwd: backendDir,
    stdio: 'inherit',
    shell: true
});

if (result.status !== 0) {
    console.error('');
    console.error('Error: Python build failed');
    process.exit(result.status || 1);
}

// Verify output
const expectedBinary = path.join(binariesDir, getExpectedBinaryName());
if (fs.existsSync(expectedBinary)) {
    const stats = fs.statSync(expectedBinary);
    const sizeMB = (stats.size / (1024 * 1024)).toFixed(1);
    console.log('');
    console.log('========================================');
    console.log('Sidecar build successful!');
    console.log('========================================');
    console.log(`Binary: ${expectedBinary}`);
    console.log(`Size: ${sizeMB} MB`);
} else {
    console.error('');
    console.error(`Warning: Expected binary not found: ${expectedBinary}`);
    console.error('The build may have failed or output to a different location.');
}

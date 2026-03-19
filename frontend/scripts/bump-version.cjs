#!/usr/bin/env node
/**
 * Bump the app version in all three manifest files at once.
 *
 * Usage:
 *   node scripts/bump-version.cjs 1.1.0
 *   npm run version:bump -- 1.1.0
 */

const fs = require('fs');
const path = require('path');

const version = process.argv[2];
if (!version) {
  console.error('Usage: node scripts/bump-version.cjs <version>');
  console.error('Example: node scripts/bump-version.cjs 1.1.0');
  process.exit(1);
}

if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) {
  console.error(`Invalid semver: "${version}". Expected format: X.Y.Z or X.Y.Z-prerelease`);
  process.exit(1);
}

const root = path.resolve(__dirname, '..');

const files = [
  {
    rel: 'package.json',
    update(content) {
      const json = JSON.parse(content);
      const old = json.version;
      json.version = version;
      return { old, content: JSON.stringify(json, null, 2) + '\n' };
    },
  },
  {
    rel: 'src-tauri/tauri.conf.json',
    update(content) {
      const json = JSON.parse(content);
      const old = json.package.version;
      json.package.version = version;
      // Also update the window title which includes the version
      for (const win of json.tauri?.windows || []) {
        if (win.title && /^Grafyn v\d/.test(win.title)) {
          win.title = `Grafyn v${version}`;
        }
      }
      return { old, content: JSON.stringify(json, null, 2) + '\n' };
    },
  },
  {
    rel: 'src-tauri/Cargo.toml',
    update(content) {
      const match = content.match(/^version\s*=\s*"([^"]+)"/m);
      const old = match ? match[1] : '?';
      const updated = content.replace(
        /^(version\s*=\s*")[^"]+(")/m,
        `$1${version}$2`
      );
      return { old, content: updated };
    },
  },
];

console.log(`Bumping version to ${version}\n`);

for (const file of files) {
  const abs = path.join(root, file.rel);
  const content = fs.readFileSync(abs, 'utf-8');
  const { old, content: updated } = file.update(content);
  fs.writeFileSync(abs, updated, 'utf-8');
  console.log(`  ${file.rel}: ${old} -> ${version}`);
}

// Regenerate Cargo.lock so CI --locked builds don't fail
const { execSync } = require('child_process');
const cargoDir = path.join(root, 'src-tauri');
try {
  console.log('\n  Regenerating Cargo.lock...');
  execSync('cargo generate-lockfile', { cwd: cargoDir, stdio: 'pipe' });
  console.log('  src-tauri/Cargo.lock: regenerated');
} catch (e) {
  console.error('  Warning: failed to regenerate Cargo.lock:', e.message);
  console.error('  Run "cd src-tauri && cargo generate-lockfile" manually before committing.');
}

console.log('\nDone! Next steps:');
console.log(`  git add -A && git commit -m "chore: bump version to ${version}"`);
console.log(`  git tag v${version}`);
console.log(`  git push origin main --tags`);

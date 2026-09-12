import { isInstallerAsset } from './installers.js';

const RELEASES_API =
  'https://api.github.com/repos/WKJBryan/Grafyn/releases?per_page=100';
const CACHE_KEY = 'grafyn:releases:v2';
const TTL_MS = 90_000;

function readCache() {
  try {
    const raw = sessionStorage.getItem(CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!parsed || !Array.isArray(parsed.releases)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function writeCache(payload) {
  try {
    sessionStorage.setItem(CACHE_KEY, JSON.stringify(payload));
  } catch {
    /* private mode */
  }
}

async function fetchWithTimeout(url, options = {}, ms = 8000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  try {
    return await fetch(url, { ...options, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

async function fetchReleasePages() {
  const releases = [];
  for (let page = 1; page <= 5; page += 1) {
    const response = await fetchWithTimeout(`${RELEASES_API}&page=${page}`, {
      headers: { Accept: 'application/vnd.github+json' },
    });
    if (!response.ok) throw new Error(`GitHub ${response.status}`);
    const batch = await response.json();
    if (!Array.isArray(batch)) throw new Error('GitHub releases shape');
    releases.push(...batch);
    if (batch.length < 100) break;
  }
  return releases;
}

export function totalDownloads(releases) {
  return (releases || []).reduce((sum, release) => {
    const assets = Array.isArray(release.assets) ? release.assets : [];
    return (
      sum +
      assets.reduce(
        (inner, asset) =>
          inner +
          (isInstallerAsset(asset.name) ? asset.download_count || 0 : 0),
        0,
      )
    );
  }, 0);
}

export function latestRelease(releases) {
  return (
    (releases || []).find((release) => !release.draft && !release.prerelease) ||
    (releases || [])[0] ||
    null
  );
}

export async function loadReleases({ allowStale = true } = {}) {
  const cached = readCache();
  const fresh = cached && Date.now() - cached.at < TTL_MS;
  if (fresh) return cached;

  try {
    const releases = await fetchReleasePages();
    const payload = {
      at: Date.now(),
      releases,
      downloads: totalDownloads(releases),
    };
    writeCache(payload);
    return payload;
  } catch (error) {
    if (allowStale && cached) return cached;
    throw error;
  }
}

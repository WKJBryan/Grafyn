import { loadReleases, latestRelease } from './github.js';
import { FALLBACK, INSTALLERS } from './installers.js';

const REFRESH_MS = 90_000;
const STATUS_LINK_CLASS =
  'underline decoration-hairline underline-offset-4 hover:text-paper';

const reduceMotion = () =>
  window.matchMedia('(prefers-reduced-motion: reduce)').matches;

function detectPlatform() {
  const ua = navigator.userAgent || '';
  const uaLower = ua.toLowerCase();
  const platform = (
    navigator.userAgentData?.platform ||
    navigator.platform ||
    ''
  ).toLowerCase();
  const haystack = `${platform} ${uaLower}`;
  const archHint = (navigator.userAgentData?.architecture || '').toLowerCase();
  const isArm = archHint.includes('arm') || /aarch64|arm64/.test(haystack);

  if (/win/.test(haystack)) return isArm ? 'win-arm64' : 'win-x64';
  if (/mac/.test(haystack) && !/like mac/.test(uaLower)) return 'mac';
  if (/linux/.test(haystack) && !/android/.test(uaLower)) return 'linux-deb';
  return 'unknown';
}

// The static fallback stays labeled as the Releases page, not as a file.
// A "Download" label only appears once a real installer asset was matched.
function primaryLabel(id) {
  const installer = INSTALLERS.find((item) => item.id === id);
  if (!installer) return 'Get Grafyn on GitHub Releases';
  if (installer.id.startsWith('win')) {
    return `Download Grafyn for Windows (${installer.format})`;
  }
  if (installer.id === 'mac') {
    return 'Download Grafyn for macOS (.dmg, Apple Silicon)';
  }
  return `Download Grafyn for Linux (${installer.format})`;
}

function matchAssets(release) {
  const assets = Array.isArray(release?.assets) ? release.assets : [];
  return INSTALLERS.map((installer) => {
    const asset = assets.find((item) => installer.test(item.name));
    return {
      ...installer,
      href: asset?.browser_download_url || FALLBACK,
      size: asset?.size,
      found: Boolean(asset),
    };
  }).filter((item) => item.found);
}

function formatSize(bytes) {
  if (!Number.isFinite(bytes)) return null;
  return `${Math.round(bytes / 1048576)} MB`;
}

function relativeDate(iso) {
  const time = Date.parse(iso);
  if (!Number.isFinite(time)) return null;
  const days = Math.floor((Date.now() - time) / 86_400_000);
  if (days <= 0) return 'released today';
  if (days === 1) return 'released yesterday';
  if (days < 30) return `released ${days} days ago`;
  const months = Math.floor(days / 30);
  return months === 1 ? 'released 1 month ago' : `released ${months} months ago`;
}

function renderMeta(root, release, primary) {
  const meta = root.querySelector('[data-download-meta]');
  if (!meta) return;
  const parts = [
    release?.tag_name,
    formatSize(primary?.size),
    relativeDate(release?.published_at),
  ].filter(Boolean);
  if (parts.length === 0) return;
  meta.hidden = false;
  meta.textContent = parts.join(' · ');
}

function formatCount(value) {
  return new Intl.NumberFormat('en-US').format(value);
}

function tickCount(el, to) {
  const from = Number.parseInt(el.dataset.value || '0', 10) || 0;
  el.dataset.value = String(to);
  if (reduceMotion() || from === to) {
    el.textContent = formatCount(to);
    return;
  }
  const start = performance.now();
  const duration = 900;
  const step = (now) => {
    const t = Math.min(1, (now - start) / duration);
    const eased = 1 - (1 - t) ** 3;
    el.textContent = formatCount(Math.round(from + (to - from) * eased));
    if (t < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

function renderStatusLoading(root) {
  const status = root.querySelector('[data-download-status]');
  if (!status) return;
  if (status.querySelector('[data-download-count]')) return;
  status.hidden = false;
  status.setAttribute('aria-busy', 'true');
  status.textContent = 'Checking the latest release…';
}

function renderStatusSuccess(root, downloads) {
  const status = root.querySelector('[data-download-status]');
  if (!status || !Number.isFinite(downloads)) return;
  status.removeAttribute('aria-busy');
  let count = status.querySelector('[data-download-count]');
  if (!count) {
    status.textContent = '';
    const link = document.createElement('a');
    link.href = FALLBACK;
    link.className = STATUS_LINK_CLASS;
    count = document.createElement('span');
    count.dataset.downloadCount = '';
    link.append(count, document.createTextNode(' installer downloads on GitHub'));
    status.append(link);
  }
  tickCount(count, downloads);
}

function renderStatusError(root) {
  const status = root.querySelector('[data-download-status]');
  if (!status) return;
  status.hidden = false;
  status.removeAttribute('aria-busy');
  status.textContent = "Couldn't reach GitHub. ";
  const link = document.createElement('a');
  link.href = FALLBACK;
  link.className = STATUS_LINK_CLASS;
  link.textContent = 'Grab any installer from the Releases page.';
  status.append(link);
}

function renderCta(root, release) {
  const platform = detectPlatform();
  const matched = matchAssets(release);
  const primary =
    matched.find((item) => item.id === platform) ||
    (platform.startsWith('win')
      ? matched.find((item) => item.id === 'win-x64')
      : null);

  const button = root.querySelector('[data-download-primary]');
  const label = root.querySelector('[data-download-label]');
  const list = root.querySelector('[data-other-platforms]');

  if (button && label) {
    label.textContent = primaryLabel(primary?.id);
    button.setAttribute('href', primary?.href || FALLBACK);
  }

  renderMeta(root, release, primary);

  if (list) {
    const others = matched.filter((item) => item.id !== (primary?.id || ''));
    list.replaceChildren();
    for (const item of others) {
      const li = document.createElement('li');
      const a = document.createElement('a');
      a.href = item.href;
      a.className = STATUS_LINK_CLASS;
      a.textContent = item.label;
      li.append(a);
      list.append(li);
    }
    if (others.length === 0) {
      const li = document.createElement('li');
      const a = document.createElement('a');
      a.href = FALLBACK;
      a.className = STATUS_LINK_CLASS;
      a.textContent = 'All releases';
      li.append(a);
      list.append(li);
    }
  }
}

async function enhance() {
  const roots = document.querySelectorAll('[data-download-cta]');
  roots.forEach(renderStatusLoading);
  try {
    const payload = await loadReleases();
    const latest = latestRelease(payload.releases);
    roots.forEach((root) => {
      if (latest) renderCta(root, latest);
      renderStatusSuccess(root, payload.downloads);
    });
  } catch {
    roots.forEach(renderStatusError);
  }
}

enhance();
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible') enhance();
});
window.setInterval(() => {
  if (document.visibilityState === 'visible') enhance();
}, REFRESH_MS);

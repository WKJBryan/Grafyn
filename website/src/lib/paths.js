export function withBase(path = '') {
  const base = import.meta.env.BASE_URL || '/';
  const trimmed = String(path).replace(/^\//, '');
  return `${base}${trimmed}`;
}

export const GITHUB_REPO = 'https://github.com/WKJBryan/Grafyn';
export const GITHUB_RELEASES = `${GITHUB_REPO}/releases/latest`;
export const GITHUB_LICENSE = `${GITHUB_REPO}/blob/main/LICENSE`;

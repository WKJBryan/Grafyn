export const FALLBACK = 'https://github.com/WKJBryan/Grafyn/releases/latest';

export const INSTALLERS = [
  {
    id: 'win-x64',
    label: 'Windows (x64)',
    format: '.exe',
    test: (name) => /^Grafyn_.*_x64-setup\.exe$/i.test(name),
  },
  {
    id: 'win-arm64',
    label: 'Windows (ARM64)',
    format: '.exe',
    test: (name) => /^Grafyn_.*_arm64-setup\.exe$/i.test(name),
  },
  {
    id: 'mac',
    label: 'macOS (Apple Silicon)',
    format: '.dmg',
    test: (name) => /^Grafyn_.*_aarch64\.dmg$/i.test(name),
  },
  {
    id: 'linux-deb',
    label: 'Linux (.deb)',
    format: '.deb',
    test: (name) => /^grafyn_.*_amd64\.deb$/i.test(name),
  },
  {
    id: 'linux-appimage',
    label: 'Linux (AppImage)',
    format: '.AppImage',
    test: (name) => /^grafyn_.*_amd64\.AppImage$/i.test(name),
  },
];

export function isInstallerAsset(name) {
  return INSTALLERS.some((installer) => installer.test(name));
}

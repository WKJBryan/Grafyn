# Grafyn website

Marketing site for Grafyn. Static Astro build, hosted on GitHub Pages at
`https://wkjbryan.github.io/Grafyn/`.

```bash
cd website
npm install
npm run dev
```

The dev server uses the same `/Grafyn/` base path as Pages, so open
`http://localhost:4321/Grafyn/`.

```bash
npm run build
npm run preview
```

Binaries are not re-hosted. The download button fetches the latest GitHub
Release in the browser and picks the installer for the visitor's OS.

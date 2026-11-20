# Scope lock

**Date:** 2026-09-11
**Artifact:** Grafyn marketing website (exists, shipped locally in-repo)

## What is being audited

- Repo path: `website/` (Astro 7 static site)
- Screens: `/` (home), `/download`, `/privacy`, `/404`
- Local URL when previewed: `http://127.0.0.1:4321/Grafyn/`
- Intended live URL: `https://wkjbryan.github.io/Grafyn/`
- Primary files:
  - `website/src/pages/index.astro`
  - `website/src/pages/download.astro`
  - `website/src/pages/privacy.astro`
  - `website/src/pages/404.astro`
  - `website/src/components/{Nav,Footer,DownloadCta,Grain}.astro`
  - `website/src/layouts/Base.astro`
  - `website/src/styles/global.css`
  - `website/src/scripts/{main,download,github,motion,nav}.js`

## Primary user

A first-time visitor who has heard of Grafyn (or landed from GitHub) and needs to decide whether to install a local-first desktop app.

## Primary task

Understand what Grafyn is (vault + canvas + experimental twin) and download the correct installer for their OS.

## Constraints

- Brand: abyssal teal from the existing logo/splash (`#061418` ink, `#3DCFC0` seed accent)
- Stack: Astro + Tailwind v4, GitHub Pages under `/Grafyn/`
- No pricing, no signup, no hosted backend
- Honesty rule: twin is experimental; do not invent social proof
- GPL/AGPL license files currently disagree; site must not hardcode a license name

## Reference / peers (not to copy)

Obsidian, Mem.ai, Linear-style AI SaaS landings, GitHub project Pages.

## Out of this audit

The Tauri desktop app UI (`frontend/`). This audit is the marketing site only.

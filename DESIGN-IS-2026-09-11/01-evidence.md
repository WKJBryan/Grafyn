# Evidence

Consolidated from five evidence subagents. No scores in this file.

## Structural (home unless noted)

Sources: `website/src/pages/index.astro` 1–233; `download.astro` 1–60; `privacy.astro` 1–69; `404.astro` 1–23; `Nav.astro` 1–120; `Footer.astro` 1–19; `DownloadCta.astro` 1–62; `Base.astro` 1–42; `download.js` 1–167.

- **Interactive elements on home (static DOM): 27** (22 `<a>` + 1 `<button>` + 2 `<details>` + 2 `<summary>`). Chrome 15 of those (skip, nav, overlay, footer) appear on every route. File:line inventory in structural subagent report.
- **Download page: 20.** Privacy / 404: **16** each.
- **Max nesting depth: 12** (`html` → … → `i.ph-arrow-up-right` in hero CTA). `Base.astro:18` through `DownloadCta.astro:16`.
- **Repeated-affordance groups on home: 6**
  1. Download/install ×5 (`Nav.astro:27`, overlay `Nav.astro:77`, hero `DownloadCta.astro:9` via `index.astro:35`, bottom `index.astro:229`, `Footer.astro:12`)
  2. Source/GitHub ×4 (`Nav.astro:40,87`, `DownloadCta.astro:21`, `Footer.astro:15`)
  3. Privacy ×4
  4. Releases/download-count URL ×5 (`DownloadCta.astro:30,56`, `index.astro:217`, bottom `DownloadCta.astro:56`, `Footer.astro:14`)
  5. Other-platforms disclosure ×2
  6. Home destination ×2 (logo + overlay)
- **`DownloadCta` instances on home: 2** (`index.astro:35` and `:229`)
- **Dead data-attrs: 2** — `data-hero` `index.astro:7`, `data-hero-art` `index.astro:14` (unused by `motion.js`)
- **Unused imports: 0**
- Three sticky `min-h-[100dvh]` articles after a `100dvh` hero (`index.astro:7,102,124,146`). `canvas.jpg` and `twin.jpg` each used twice (bento + sticky).

## Visual

Sources: `global.css` 1–417; built `dist/_astro/Base.Dzv_X7gs.css`; screenshots `website/screenshots/home-desktop.png`, `home-mobile.png`, `home-mid.png`, `download-desktop.png`.

- **Spacing:** 20+ steps including 3, 4, 6, 8, 10, 12, 12.8, 16, 20, 21.6, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160 px. Footer `md:px-8` (32) vs content `lg:px-16` (64).
- **Type:** 11 sizes (14, 15, 16, 18, 20, 24, 30, 36, 40, 48, 60 px). Satoshi 700 loaded (`global.css:21–27`) with no `font-bold` usage.
- **Colors used: 7 hex hues.** Tokens: ink `#061418`, abyss `#021a20`, surface `#0c2a32`, seed `#3dcfc0`, paper `#e8f2f2`, mute `#8fb3b6`, hover `#5ee0d2`. `--color-graph: #7a86e8` defined `global.css:36` and unused.
- **Contrast (sRGB WCAG):**
  - paper on ink **16.43:1**
  - mute on ink **8.29:1**
  - ink on seed **9.71:1**
  - mute on surface **6.67:1**
  - hairline (seed 14%) on ink **~1.3:1** (non-text 1.4.11 fail)
- **States:** empty = hidden blank count; loading = `aria-busy` + `hidden`, no spinner; **error missing** (`download.js:155–157` empty catch); success = unhide number; focus = `:focus-visible` seed ring `global.css:70–73`; **disabled missing**.

## Copy and honesty

Sources: all page/component strings (full list in copy subagent). Behavior: `download.js`, `github.js:62–69`, `tauri.conf.json` updater + `externalBin`.

- **Inflations (2+):** “A vault that organizes itself” `index.astro:52`; “records how you think” `:32`; “Nothing is hosted. Delete the folder and it is gone.” `:180`; “Or nothing external, if you run Ollama.” `:201`; “One desktop binary” `download.astro:17`; “That is the whole footprint.” `privacy.astro:58`; live `{n} downloads` as if people/installs.
- **Dark patterns:** no account, no scarcity, no confirmshaming. Mild social-proof via live-dot + all-asset GitHub total. Primary CTA looks like a file download; static href is GitHub Releases (`DownloadCta.astro:12–14`); arrow icon signals leaving.
- **Label→behavior:** `Download Grafyn` → `/releases/latest` until JS; Linux button → always `.deb`; macOS button → Apple Silicon `.dmg` only; `{n} downloads` = sum of every release asset including `.sig` / updater zips (`github.js:62–69`); `All releases` → latest releases URL not `/releases`.
- **Honesty that holds:** no Grafyn account (`privacy.astro:16–17`); twin experimental (`index.astro:92–93`); keychain (`privacy.astro:40–41`); auto-update exists (`index.astro:226`).
- **Jargon without first-use gloss:** twin, canvas, wikilinks, topic hubs, MCP, distillation, OpenRouter, Ollama, vault, evidence-linked records, no-train.

## Weight and friction

Sources: `website/dist` file sizes; `index.astro` imgs; `global.css`; `download.js:161–166`; `motion.js`.

- **Initial JS:** 5662 bytes uncompressed (`dist/_astro/Base.astro_…CyKdC06d.js`). Gzip not measured on the wire.
- **CSS:** 98126 bytes. **Art JPEGs:** 755162 bytes, 0 `loading="lazy"` / `srcset`.
- **Home network requests (method: tags + CSS fonts + 1 API): 21** (1 html, 1 css, 1 js, 9 `@font-face`, 8 `<img>` tags, 1 GitHub releases fetch). Unique image URLs: 6.
- **TTI:** ~2000 ms estimated from ~461 KB critical path on Fast 4G. Not Lighthouse.
- **Idle animations: 2** — `hero-drift` 28s infinite `global.css:324`; `live-pulse` 2.4s infinite `global.css:362`. Grain is static. `hero-enter` is load-only. `will-change: transform` always on `.hero-art-wrap` `global.css:315`.
- **Modals/toasts on load: 0**
- **`prefers-reduced-motion`: yes** (`global.css:47–51,206–215,399–416`; `motion.js`; `download.js:34–35`)
- **`prefers-color-scheme`: no.** Dark-only `#061418`.
- Visible-tab refetch every 90s (`download.js:161–166`).

## Accessibility

Sources: `Base.astro:33,36`; `Nav.astro:11–95`; `DownloadCta.astro:7–61`; `Footer.astro:5–18`; `global.css` focus and overlay; `nav.js`.

- Skip link **yes** `Base.astro:33`
- Landmarks: 1 header, 2 nav (Primary, Footer), 1 main, 1 footer
- Text contrast tokens pass AA/AAA; hairline vs ink **fails 1.4.11** (~1.3:1)
- Keyboard: primary actions reachable as native `a`/`button`/`summary`
- **Closed mobile overlay links remain in tab order** (`opacity:0; pointer-events:none` only; no `inert`/`hidden`) `Nav.astro:63`; `global.css` `.nav-overlay`
- Burger `aria-label` stays “Open menu”; no `aria-controls`; overlay not a dialog; no focus trap
- Live counts: no `aria-live`; start `hidden`
- All listed images have `alt=""` (none missing `alt`)
- `details`/`summary` native keyboard yes

## Known gaps (all agents)

- No Lighthouse / computed-style dump from DevTools
- Hairline contrast is token math, not screenshot pixels over photography
- GitHub payload size and Intel Mac user path not live-tested in this audit

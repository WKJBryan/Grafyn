````
/make-plan Redesign the Grafyn marketing website. Current design failed audit at 16/30 with critical gaps in principles #5 unobtrusive (1), #6 honest (1), #8 thorough (1), #10 as little design as possible (1).

Verdict paragraph (quoted from 03-verdict.md):
> The marketing site failed a Rams audit at 16/30: the first screen can download, but the rest is a duplicated, trend-heavy story with honesty gaps, so the form should be rebuilt around one install path rather than polished in place.

Why redesign and not refine: Total is 16/30, below the refine floor of 20, and the three-product story is told twice (bento then three full-viewport sticky chapters) while install affordances appear five times.

Preserve from current design (MUST be non-empty — at minimum, name the brand tokens):
- Color tokens ink `#061418`, abyss `#021a20`, surface `#0c2a32`, seed `#3DCFC0`, paper `#E8F2F2`, mute `#8FB3B6` (`website/src/styles/global.css:32–38`)
- Satoshi + IBM Plex Mono pairing (`global.css:4–27`)
- Logo mark `website/public/mark.png` used in `Nav.astro:14–17`
- OS-aware GitHub installer matching (`website/src/scripts/download.js` INSTALLERS + `detectPlatform`)
- Privacy facts that are true: no Grafyn account (`privacy.astro:16–17`), twin experimental (`index.astro:92–93`), keychain (`privacy.astro:40–41`)
- Routes `/`, `/download`, `/privacy` and GitHub Pages `base: '/Grafyn/'`

Discard (MUST be non-empty — name the structural patterns causing the failures):
- Second home `DownloadCta` plus duplicate Other platforms. Evidence: `index.astro:35` and `:229`. Caused failure on principle #10.
- Three sticky `min-h-[100dvh]` Write/Compare/Review chapters that repeat the bento. Evidence: `index.astro:101–166` vs `:43–96`. Caused failure on principles #10 and #5.
- Idle `hero-drift` and `live-pulse`. Evidence: `global.css:324,362`. Caused failure on principles #5 and #9.
- All-asset GitHub total presented as “downloads” with a live dot. Evidence: `github.js:62–69`, `index.astro:217–220`. Caused failure on principle #6.
- Absolute claims “Nothing is hosted”, “One desktop binary”, “whole footprint”. Evidence: `index.astro:180`; `download.astro:17`; `privacy.astro:58`. Caused failure on principle #6.

Top 3–5 moves from the audit (verbatim):
1. **#10 as little design as possible:** Tell vault / canvas / twin once. Keep a single download island. Cut the second `DownloadCta`, the extra live-count, the extra Other platforms, and the three `100dvh` sticky chapters that repeat the bento. Evidence: `index.astro:35` and `:229`; `index.astro:43–96` vs `:101–166`; Structural repeated-affordance groups.
2. **#6 honest:** Label the GitHub number as GitHub asset downloads, or sum installer files only. Say what still leaves the machine (updater, optional OpenRouter). Do not claim “one binary” or “nothing is hosted / delete the folder.” Evidence: `github.js:62–69`; `index.astro:180,201`; `download.astro:17`; `privacy.astro:58`.
3. **#8 thorough:** Give the GitHub path a visible loading and error state. If the API fails, say so and keep the Releases fallback labeled as a page, not as a silent fake download. Evidence: `download.js:155–157`; `DownloadCta.astro:31–35`.
4. **#5 unobtrusive:** Drop idle ken-burns and the live pulse. Keep motion for user-caused state (menu, details, count tick on data). Evidence: `global.css:324,362`; `motion.js:57–71`.
5. **#4 understandable:** On first mention, gloss twin, canvas, and MCP in plain words. Put the actual format on the OS button (Windows installer, macOS Apple Silicon, Linux .deb). Evidence: `index.astro:89,207`; `download.js:51,56–58`.

Redesign principles in priority order:
1. **#2 Useful** — One obvious install action in the first viewport that is either a real file or clearly “GitHub Releases”; Other platforms once.
2. **#6 Honest** — Every claim maps to disk, GitHub, or OpenRouter; count method stated in the label.
3. **#10 As little design as possible** — One pass at the three products, then privacy, then install. No second theatre.

Deliverables for the plan:
- New information architecture (not derived from old): home section list with one job each
- New primary flow (low-fi, labeled, compared side-by-side to current hero → bento → sticky ×3 → local-first → CTA)
- States checklist (empty, loading, error, success, focus, disabled) for DownloadCta / GitHub fetch
- Migration path for users currently on the old design (same URLs `/`, `/download`, `/privacy`; no new marketing routes)
- Cutover criteria (when is the old design retired): replace `website/src/pages/index.astro` in place; do not keep a flag

Anti-patterns to guard against (specific to REDESIGN):
- Porting old structure under new styling
- Keeping both designs behind a flag indefinitely
- Redesigning to follow a trend rather than the principles above
- Treating the Preserve list as optional — it must be filled before this handoff is valid
````

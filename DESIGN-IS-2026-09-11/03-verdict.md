# Verdict

**REDESIGN**

The marketing site failed a Rams audit at 16/30: the first screen can download, but the rest is a duplicated, trend-heavy story with honesty gaps, so the form should be rebuilt around one install path rather than polished in place.

Highest-leverage moves:

1. **#10 as little design as possible:** Tell vault / canvas / twin once. Keep a single download island. Cut the second `DownloadCta`, the extra live-count, the extra Other platforms, and the three `100dvh` sticky chapters that repeat the bento. Evidence: `index.astro:35` and `:229`; `index.astro:43–96` vs `:101–166`; Structural repeated-affordance groups.

2. **#6 honest:** Label the GitHub number as GitHub asset downloads, or sum installer files only. Say what still leaves the machine (updater, optional OpenRouter). Do not claim “one binary” or “nothing is hosted / delete the folder.” Evidence: `github.js:62–69`; `index.astro:180,201`; `download.astro:17`; `privacy.astro:58`.

3. **#8 thorough:** Give the GitHub path a visible loading and error state. If the API fails, say so and keep the Releases fallback labeled as a page, not as a silent fake download. Evidence: `download.js:155–157`; `DownloadCta.astro:31–35`.

4. **#5 unobtrusive:** Drop idle ken-burns and the live pulse. Keep motion for user-caused state (menu, details, count tick on data). Evidence: `global.css:324,362`; `motion.js:57–71`.

5. **#4 understandable:** On first mention, gloss twin, canvas, and MCP in plain words. Put the actual format on the OS button (Windows installer, macOS Apple Silicon, Linux .deb). Evidence: `index.astro:89,207`; `download.js:51,56–58`.

No principle scored 0. Total is under 20, so the rule is redesign, not refine. Preserve the brand (ink/seed/Satoshi/mark) and the OS-aware GitHub installer matcher. Discard the duplicated scroll architecture.

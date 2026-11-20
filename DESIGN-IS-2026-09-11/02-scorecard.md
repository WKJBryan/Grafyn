# Scorecard

Scoring: 0–3 integer, worst instance not mean, tie goes lower. Max 30.

1. Good design is innovative — Score: 2/3
   Evidence: OS-aware GitHub asset matching (`download.js:37–52,107–123`) is a real product-specific improvement on a stock dark landing (island nav `Nav.astro:11–12`, bento `index.astro:41–96`, sticky stack `index.astro:101–166`). 01-evidence.md Structural + Weight.
   Justification: It refreshes a common 2024–26 SaaS landing kit with a GitHub-native installer path, but it does not introduce a pattern unseen in peer product sites.

2. Good design makes a product useful — Score: 2/3
   Evidence: Primary download is in the first viewport (`index.astro:35` + `DownloadCta.astro:9`) but the static href is GitHub Releases until JS (`DownloadCta.astro:12–14`); home also ships a second full install widget (`index.astro:229`) plus five download-intent links. 01-evidence.md Structural.
   Justification: The task can complete from the hero, yet adjacent chrome and a JS-gated file URL add steps versus a single always-correct installer action.

3. Good design is aesthetic — Score: 2/3
   Evidence: One teal system (ink/seed/paper/mute) and coherent hero/bento screenshots; inconsistencies include 11 type sizes, 20+ spacing steps, footer `md:px-8` vs content `lg:px-16`, unused Satoshi 700. 01-evidence.md Visual.
   Justification: The page reads as one brand, not noise, but the scale is not a single visible rhythm so it is not a 3.

4. Good design makes a product understandable — Score: 2/3
   Evidence: Primary controls name themselves (Download, View source, Other platforms, Privacy). Jargon (twin, canvas, MCP, OpenRouter, wikilinks) appears without first-use gloss (`index.astro:54–56,89–93,207–208`). Linux CTA says “for Linux” and always picks `.deb` (`download.js:51,58`). 01-evidence.md Copy, Accessibility.
   Justification: A first-time user can name the download button; one control still needs a qualifier and body copy is not self-explanatory.

5. Good design is unobtrusive — Score: 1/3
   Evidence: Idle `hero-drift` 28s (`global.css:324`), live-pulse dots (`global.css:362`), grain overlay (`Grain.astro:3`), magnetic buttons (`motion.js:57–71`), and three extra `100dvh` sticky chapters after a full-viewport hero (`index.astro:102,124,146`). 01-evidence.md Structural, Visual, Weight.
   Justification: Decoration and pacing compete with the install task instead of receding behind it.

6. Good design is honest — Score: 1/3
   Evidence: Two-plus inflations: “Nothing is hosted. Delete the folder and it is gone.” (`index.astro:180`) vs updater host; “One desktop binary” (`download.astro:17`) vs `grafyn-mcp` sidecar; `{n} downloads` is all GitHub assets including `.sig` (`github.js:62–69`). Twin experimental is honestly labeled (`index.astro:92–93`). 01-evidence.md Copy.
   Justification: No deceptive checkout, but more than one inflated claim, so not a 2.

7. Good design is long-lasting — Score: 1/3
   Evidence: Dated markers: glass island nav (`global.css:230–245`), bento tiles (`index.astro:41–96`), infinite ken-burns hero (`global.css:324–333`), grain overlay (`global.css:91–99`). 01-evidence.md Visual.
   Justification: Two to three trend markers are present; the teal seed mark would last, the chrome around it would date.

8. Good design is thorough down to the last detail — Score: 1/3
   Evidence: GitHub fetch error is an empty catch (`download.js:155–157`); no disabled state in `website/src`; loading is hide-until-ready with no spinner (`DownloadCta.astro:31–35`); closed overlay links stay in tab order (`Nav.astro:63`). Focus ring exists (`global.css:70–73`). 01-evidence.md Visual, Accessibility.
   Justification: Two to three states are missing or rough (error, disabled, loading), which is a 1 not a 0.

9. Good design is environmentally friendly — Score: 2/3
   Evidence: JS 5662 bytes; motion gated by `prefers-reduced-motion` (`global.css:399–416`); idle animations still run by default (`hero-drift`, `live-pulse`); art 755162 bytes with no `loading="lazy"`; dark-only, no `prefers-color-scheme`. 01-evidence.md Weight.
   Justification: JS is small and motion can be turned off, but idle animation and ungated images block a 3.

10. Good design is as little design as possible — Score: 1/3
   Evidence: Removable or duplicated: second `DownloadCta` (`index.astro:229`), View source already in nav (`DownloadCta.astro:21` vs `Nav.astro:40`), second live count (`index.astro:217` vs CTA), second Other platforms, grain, sticky stack restating the bento’s three products (`index.astro:101–166` vs `43–96`). 01-evidence.md Structural.
   Justification: Three to five elements can come out without breaking download; the page is not yet only essentials.

**Total: 16 / 30**

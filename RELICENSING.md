# Relicensing record

## Decision

On 2026-08-29, the repository owner authorized changing the Grafyn open client and local core to the Mozilla Public License 2.0 (MPL-2.0). The approved companion design records the open/proprietary boundary in [`docs/superpowers/specs/2026-08-29-grafyn-companion-system-design.md`](docs/superpowers/specs/2026-08-29-grafyn-companion-system-design.md).

Before this change, the root `LICENSE` contained the GNU Affero General Public License, version 3. Package metadata was inconsistent: the frontend and Rust package declared `GPL-3.0-only`, while the E2E package declared `ISC`. This record describes the repository evidence used for the change; it does not reinterpret those inconsistencies or revoke grants already made. Earlier recipients retain the license grants that applied to the versions they received.

## Provenance audit at the change boundary

The audit was run against commit `edae5e1b0d5e3c9ae0726e771d3c0fc7bafc9530`, immediately before the governance changes.

- The reachable history contained 212 commits: 169 non-merge commits and 43 merge commits.
- All 169 non-merge commits were inspected with their changed paths and resolved to two owner-associated Git author identities: 141 commits by `WKJBryan <bryanwangkangjie.03@gmail.com>` across 403 unique paths, and 28 commits by `Bryan <227426514+WKJBryan@users.noreply.github.com>` across 154 unique paths.
- Of the 43 merge commits, 34 had a tree identical to parent 2, one had a tree identical to parent 1, five were clean merge-only commits with no combined-diff paths, and three contained combined-diff resolutions. Those three resolutions were authored under the same two owner-associated identities and affected only the paths reported by `git diff-tree --cc`.
- The only other author identity, `Building0 <building0.sutd@gmail.com>`, appeared on two merge commits (`9f5468b8` and `df54bf8d`). Each merge tree was identical to its second parent, so neither commit introduced a unique authored tree. Every reachable non-merge commit behind them was already included in the author/path audit above.
- The 279 tracked files contained no submodules and no tracked `vendor`, `vendored`, `third-party`, `external`, `deps`, or generated/minified-source directory matching the audit patterns. A repository-wide notice scan found no embedded SPDX, copyright, license, copied-source, adapted-source, or third-party source notice to preserve. The matches for “derived from” and “Imported from” were internal code/runtime wording, not external attribution.
- Package-manager dependencies remain governed by the licenses recorded by their upstream packages and lockfile metadata. They are not relicensed by this repository change.

The reproducible audit used `git rev-list`, `git log --no-merges --name-only`, tree comparisons with `git rev-parse <commit>^{tree}`, `git diff-tree --cc`, `git ls-files`, and `git grep` notice patterns. Git identity is evidence, not legal proof. The audit supports the owner's authorization for code they represent they control; it is not a warranty about authorship or a substitute for legal advice. If contrary provenance evidence is found, the affected material must remain under its existing terms or be cleanly replaced rather than silently relicensed.

## License boundary after the change

- The Grafyn client and local core are distributed under MPL-2.0. The root [`LICENSE`](LICENSE) and [`LICENSES/MPL-2.0.txt`](LICENSES/MPL-2.0.txt) contain Mozilla's unmodified official text.
- [`LICENSES/Apache-2.0.txt`](LICENSES/Apache-2.0.txt) retains the unmodified official Apache License 2.0 text for a protocol package that explicitly declares `Apache-2.0 OR MPL-2.0`. Merely including that text does not dual-license the rest of the repository.
- Existing third-party dependencies, assets, and notices remain under their original licenses.
- A future production hosted service, including relay operations, billing, quotas, and abuse controls, is a separate proprietary boundary. No such service is available in this repository today.

Canonical texts were retrieved from [Mozilla's MPL 2.0 plain-text publication](https://www.mozilla.org/media/MPL/2.0/index.txt), the [Apache Software Foundation's Apache 2.0 text](https://www.apache.org/licenses/LICENSE-2.0.txt), and the [Developer Certificate of Origin 1.1](https://developercertificate.org/).

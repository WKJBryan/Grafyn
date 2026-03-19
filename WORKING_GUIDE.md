# Working Guide

This is the day-to-day workflow for Grafyn after the release hardening changes on March 19, 2026.

## Ground Rules

- Do not push directly to `main`. `main` is protected and expects pull requests plus passing checks.
- Do not move or delete `v*` tags. Release tags are protected and immutable releases are enabled.
- Do not hand-edit versions for a release. Use the release scripts in `frontend/package.json`.
- Do not push a release tag first and hope CI figures it out. The tag is the final step, not the first step.

## Normal Development Flow

1. Create a branch from `main`.
2. Make your changes.
3. Run the checks that match your change.

```bash
cd frontend
npm run test:run
npm run lint

cd src-tauri
cargo test
```

4. Push the branch and open a PR into `main`.
5. Wait for GitHub checks to pass.
6. Merge the PR.

## What CI Now Protects

Every PR and every push to `main` now runs:

- Test suite in `.github/workflows/test.yml`
- Release preflight in `.github/workflows/test.yml`
- Cross-platform release smoke builds in `.github/workflows/release-smoke.yml`

That means lockfile drift, version drift, and release-only build problems should be caught before a tag is ever pushed.

## Release Workflow

Use a fresh version every time. Do not reuse an old tag.

### Step 1: Prepare the release on a branch

Create a short-lived release branch from the latest `main`, then run:

```bash
cd frontend
npm run release:prepare -- 0.1.4
```

This does all of the risky work locally:

- bumps the app version in all release manifests
- regenerates `Cargo.lock` with `cargo generate-lockfile`
- validates the lockfile against all 4 release targets and both Cargo graphs
- creates a release prep commit

Then:

1. Push that branch.
2. Open a PR into `main`.
3. Let CI pass.
4. Merge the PR.

### Step 2: Cut the tag from clean `main`

After the release PR is merged:

```bash
git switch main
git pull --ff-only
cd frontend
npm run release:tag -- 0.1.4
git push origin v0.1.4
```

When the version already matches `0.1.4`, `npm run release:tag -- 0.1.4` does the safe final step:

- verifies the tree is clean
- re-runs release verification
- creates the annotated `v0.1.4` tag on the current `main` commit

Pushing that tag triggers `.github/workflows/release.yml`.

## What the Tagged Release Workflow Does

The release workflow now:

1. verifies the tagged commit again
2. reuses one draft release for that tag if it exists
3. deletes stale assets from that draft before rebuilding
4. builds all release targets
5. verifies the updater artifacts are complete
6. publishes the release
7. generates one canonical `latest.json`
8. uploads release assets and updater metadata to Cloudflare R2
9. verifies the updater endpoint returns the expected version

If a build leg fails before publish and the workflow created a fresh draft, it cleans that draft up automatically.

## The Release Commands

From `frontend/`:

```bash
npm run release:verify
```

Use this any time you want to sanity-check release readiness. It fails if:

- the git tree is dirty
- versions are out of sync
- `Cargo.lock` does not satisfy the release targets under `--locked`

```bash
npm run release:prepare -- X.Y.Z
```

Use this on a release branch to create the version bump commit that will go through a PR.

```bash
npm run release:tag -- X.Y.Z
```

Use this after the release PR is merged and `main` already contains `X.Y.Z`. It creates the final tag from clean `main`.

## If Something Fails

### `release:verify` fails

Fix the repo before doing anything else.

Common causes:

- uncommitted files
- mismatched version numbers
- `Cargo.lock` not matching the manifest and release feature sets

### The release workflow says a published release already exists

Do not reuse that version. Cut the next one instead.

### The release workflow finds duplicate drafts for the same tag

Clean the duplicate drafts in GitHub, then rerun. This should only happen with old history from before the hardening changes.

### Cloudflare updater verification fails

Treat that as a release problem, not a cosmetic problem. The workflow is checking the same endpoint the app uses for auto-update.

## Ongoing Maintenance

- Review weekly Dependabot PRs for Cargo, npm, and GitHub Actions.
- Watch the weekly `latest-deps` workflow for dependency drift.
- Keep release secrets available under the `release` environment when you rotate them.

## Short Version

- Feature work goes through PRs.
- Release prep goes through a PR.
- The tag is cut only after the release prep PR is merged.
- Never reuse a release version.

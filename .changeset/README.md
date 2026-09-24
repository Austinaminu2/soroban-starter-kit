# Changesets

This directory contains changesets for the soroban-starter-kit project. Changesets are a way to manage versions and changelogs in a monorepo.

## Adding a Changeset

When you make a change that affects one or more contract crates, create a changeset file:

```bash
npx changeset add
```

This will guide you through:
1. Selecting which packages are affected
2. Choosing the bump type (major, minor, patch)
3. Writing a summary of the changes

Each changeset is stored as a markdown file in this directory with a unique ID, e.g., `fancy-armadillos-12345.md`.

## Changeset Format

A changeset file looks like:

```markdown
---
"soroban-token-template": patch
"soroban-escrow-template": minor
"soroban-dao-template": patch
---

Fixed critical issue in token transfer validation.

Added new utility functions for escrow contracts.
```

The frontmatter specifies which packages are affected and their bump types:
- **patch**: Bug fixes and minor improvements (e.g., 0.1.0 → 0.1.1)
- **minor**: New features that are backward compatible (e.g., 0.1.0 → 0.2.0)
- **major**: Breaking changes (e.g., 0.1.0 → 1.0.0)

The body contains a concise description of what changed (appears in the CHANGELOG).

## Release Process

When you're ready to release:

1. **Create a release PR**: The changeset bot will automatically create a PR that:
   - Bumps versions in all affected package Cargo.toml files
   - Generates/updates CHANGELOG.md files
   - Removes changeset files

2. **Review and merge**: Once approved and CI passes, merge the release PR

3. **Automated release**: The CI workflow will automatically:
   - Tag the release with version numbers
   - Create GitHub releases
   - Publish crates to crates.io (if configured)

## GitHub Actions Integration

The project uses the `changesets/action@v1` workflow to:
- Monitor PR titles and labels
- Auto-create release PRs when changesets are detected
- Detect release-ready PRs and publish updates

See `.github/workflows/release.yml` for the full configuration.

## Tips

- **One changeset per PR**: Create a single changeset file per PR, even if multiple packages are affected
- **Be descriptive**: Write clear summaries; they appear in the CHANGELOG
- **Semver discipline**: Follow semantic versioning carefully
- **Skip for docs/CI**: Non-package changes (docs, CI, internal tooling) don't need changesets

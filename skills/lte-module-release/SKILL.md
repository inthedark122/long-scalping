---
name: lte-module-release
description: Create a new LTE module version, build and deploy it, update any existing bot groups to the new module version, and commit only the release changes. Use when the user asks to make a new version, deploy a module to LTE, refresh bot groups, or record the release in git.
---

# LTE Module Release

Use this skill for the full release cycle of the module in this repo.

## Scope

- Bump the module version.
- Build and deploy the module through LTE MCP.
- Update bot groups that already exist to the new module version.
- Commit only the release-related files.

Do not touch unrelated dirty files.

## Workflow

1. Check the current git state and identify the next version from `module/Cargo.toml` and `module/CHANGELOG.md`.
2. Update the release files together:
   - `module/Cargo.toml`
   - `module/Cargo.lock`
   - `module/module.manifest.template.json`
   - `strategy.json`
   - `module/CHANGELOG.md`
3. Validate the module locally with `cargo check --bin module_entry --locked` from `module/`.
4. Package the `module/` crate as a `.tar.gz`, upload it with LTE MCP, submit the build, and poll until the build is `Success`.
   - Exclude non-release files such as `target/`, `tmp/`, caches, and other unrelated workspace artifacts when creating the archive.
5. Update any existing bot groups that should track the release. Patch both:
   - `strategy.module_version`
   - `strategy.professional.dynamic_module.module_version`
   Also refresh the `start` action description if it includes the version.
6. Commit only the release changes with a clear release message, typically:
   - `chore(release): bump module to vX.Y.Z`

## Guardrails

- Never assume the deploy succeeded until LTE build status is `Success`.
- Never rewrite unrelated files.
- If bot groups do not exist, skip the update step rather than creating new ones unless the user explicitly asks.
- Keep the commit focused on the release files only.

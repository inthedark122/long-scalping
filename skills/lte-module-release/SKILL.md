---
name: lte-module-release
description: Create a new LTE module version, build and deploy it, optionally update configured bot groups, and commit only the release changes. Use when the user asks to make a new version, deploy a module to LTE, refresh configured bot groups, or record the release in git.
---

# LTE Module Release

Use this skill for the full release cycle of the module in this repo.

## Scope

- Bump the module version.
- Build and deploy the module through LTE MCP.
- Optionally update bot groups described by the tracked project configuration.
- Commit only the release-related files.

Do not touch unrelated dirty files.

## Bot-group configuration

The configuration schema is bundled with this skill at [`references/bot-groups.schema.json`](references/bot-groups.schema.json).
The project configuration is `config/bot-groups.json`.

- Validate `config/bot-groups.json` against the bundled schema before using it.
- If `config/bot-groups.json` does not exist, skip all bot-group discovery, updates, and creation.
- Treat `config/bot-groups.json` as the desired configuration and operate only on its listed groups.
- Keep API keys, credentials, and LTE account secrets out of the configuration; LTE stores API-key associations separately.
- Update only entries with `release.update_module_version: true`.
- Update both `strategy.module_version` and `strategy.professional.dynamic_module.module_version` when those paths exist.
- Refresh the configured start action only when `release.refresh_start_action_description: true` and its description contains an older release version.
- Never create a bot group unless an entry explicitly sets `release.create_if_missing: true` and contains complete settings.
- An absent configuration or an empty `bot_groups` array must never result in a bot-group creation.

## Workflow

1. Check the current git state and identify the next version from `module/Cargo.toml` and `module/CHANGELOG.md`.
2. Update the release files together:
   - `module/Cargo.toml`
   - `module/Cargo.lock`
   - `module/module.manifest.template.json`
   - `strategy.json`
   - `module/CHANGELOG.md`
   - `config/bot-groups.json` when its configured module versions or release descriptions change
3. Validate the module locally with `cargo check --bin module_entry --locked` from `module/`.
4. Package the `module/` crate as a `.tar.gz`, upload it with LTE MCP, submit the build, and poll until the build is `Success`.
   - Exclude non-release files such as `target/`, `tmp/`, caches, and other unrelated workspace artifacts when creating the archive.
5. If `config/bot-groups.json` exists, validate and read it. Update only configured groups. Patch both:
   - `strategy.module_version`
   - `strategy.professional.dynamic_module.module_version`
   Also refresh the configured `start` action description when enabled and it includes an older version.
   If the configuration is absent or has no entries, skip this step completely; do not create bot groups.
6. Commit only the release changes with a clear release message, typically:
   - `chore(release): bump module to vX.Y.Z`

## Guardrails

- Never assume the deploy succeeded until LTE build status is `Success`.
- Never rewrite unrelated files.
- If the project configuration is absent, do not inspect or create bot groups.
- If configured bot groups do not exist, skip them unless their entry explicitly opts into creation with complete settings.
- Never infer bot-group targets from live account data when they are not present in `config/bot-groups.json`.
- Keep the commit focused on the release files only.

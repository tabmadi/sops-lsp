# Agent guide

Tool-agnostic guide for any coding agent (Codex, Cursor, Claude Code, or another) working in this repo. `AGENTS.md` is the one standard: an agent either reads it or it does not — the repo carries no per-tool shim files (`CLAUDE.md`, `.cursor/rules/`, etc.). A tool that ignores `AGENTS.md` is a limitation of that tool, not something the repo works around.

## The one rule that outranks this file

**Humans are the first developers. [README.md](README.md) outranks this file.** It holds what this project is, its status, and the upstream constraint that shapes the design. This file holds only operational hints: how to navigate, build, and run the repo.

## What this repo is

A language server for SOPS-encrypted files, plus the Zed extension that starts it. Two crates:

| Path | Target | Built by |
| --- | --- | --- |
| `src/` | native binary `sops-lsp` | `cargo build`, and the release workflow for each platform |
| `editors/zed/` | `wasm32-wasip2` cdylib | Zed itself, on `zed: install dev extension` |

`editors/zed` is excluded from the workspace and has its own lockfile. Every task that spans both crates names both — a bare `cargo` command at the root does not reach the extension.

## The rules that hold this design together

- **Never reimplement SOPS.** The server shells out to the `sops` binary. Key discovery, MAC verification, and the file format are its problem, and a second implementation of any of them is a security defect waiting to be found.
- **Plaintext does not touch the disk.** Decryption goes into the buffer over `workspace/applyEdit`. A design that writes a plaintext sidecar, or lets the editor flush plaintext and re-encrypts afterwards, is rejected: the honest answer until Zed supports `textDocument/willSaveWaitUntil` is that the write direction is unfinished, and README.md says so.
- **The server never logs a decrypted value**, including at debug level, and including in an error it returns to the client.
- **Anything added to `[tools]` in `mise.toml` is pinned to an exact version.** Rust is the exception and is pinned by `rust-toolchain.toml`, because rustup is what Zed invokes.

## Working in the repo

- The task runner is `mise` (root `mise.toml`); commands are `mise run <task>`. `mise run setup` installs the git hooks.
- `mise run check` runs format, lint, and test together; `mise run pre-commit` is what the git hook calls. `mise run lint` covers both crates, including a clippy pass against the wasm target.
- Lint policy lives in `Cargo.toml`'s `[lints]` tables, not in the task or in CI flags. A lint exception goes there with the reason, so `cargo clippy` alone reproduces CI.
- CI is [.github/workflows/pr-checks.yml](.github/workflows/pr-checks.yml): PR title validated as a Conventional Commit, then `mise run lint` and `mise run test`. [.github/workflows/release.yml](.github/workflows/release.yml) builds the four release assets on a `v*` tag, and the extension resolves them by name — renaming an asset breaks installation for everyone.
- `mise run act` replays the pull request workflow locally with [act](https://github.com/nektos/act). It reads secrets from `.env` — copy `.env.example` first.

## Commits

- **Conventional Commits, enforced.** `cog verify` runs on `commit-msg` and `cog check` on `pre-push`, so a malformed message is rejected locally before CI sees it.
- Commit messages are a title only — no body, no footer.
- Never push unless asked to.

# sops-lsp

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

A language server for [SOPS](https://github.com/getsops/sops)-encrypted files, and a [Zed](https://zed.dev) extension that runs it.

Editing a SOPS file today means `sops edit` in a terminal, or decrypting to a file you have to remember to delete. This server moves that into the editor: the ciphertext in the buffer is replaced with plaintext over `workspace/applyEdit`, and the encryption stays where it belongs — in the `sops` binary, which this server shells out to and never reimplements.

## Status

Early. The server recognises a SOPS-encrypted document and reports it; decryption and re-encryption are the next milestones.

The write direction has an upstream dependency worth knowing before you file a bug. LSP's `textDocument/willSaveWaitUntil` is the request that lets a server rewrite a buffer *before* the editor writes it to disk, and it is the only way to re-encrypt with no plaintext ever reaching the filesystem. Zed declares `did_save` and not `will_save_wait_until` ([`crates/lsp/src/lsp.rs`](https://github.com/zed-industries/zed/blob/main/crates/lsp/src/lsp.rs)), so until that lands, any save path has a window where plaintext exists on disk. This project will not ship a design that hides that window rather than closing it.

The decrypt direction needs nothing from upstream: Zed already declares `apply_edit` and handles server-to-client `workspace/applyEdit`.

## Install

### As a Zed dev extension

```sh
git clone https://github.com/tabmadi/sops-lsp
cd sops-lsp
cargo build --release          # the server
```

Put the binary on `PATH` (`target/release/sops-lsp`), then in Zed run `zed: install dev extension` and select `editors/zed`.

The extension prefers a `sops-lsp` on `PATH` over anything it has downloaded, so a working build always wins. With no binary on `PATH` it fetches the matching release asset instead.

`sops` itself must be on `PATH`, with your keys configured as usual — the extension passes the worktree's shell environment through, so `SOPS_AGE_KEY_FILE` and friends reach the server.

## Development

```sh
mise run setup      # git hooks
mise run check      # format, lint, test
mise run build      # the server binary
mise run lint       # clippy over both crates, including the wasm target
```

Rust is pinned by [`rust-toolchain.toml`](rust-toolchain.toml) rather than by mise, because rustup is what Zed invokes when it builds the extension.

## Layout

| Path | What it is |
| --- | --- |
| `src/` | the language server, a native binary |
| `editors/zed/` | the Zed extension: a wasm32-wasip2 cdylib that locates and starts the server |

`editors/zed` is excluded from the workspace and carries its own lockfile, because Zed builds it on its own terms.

## Licence

Apache-2.0.

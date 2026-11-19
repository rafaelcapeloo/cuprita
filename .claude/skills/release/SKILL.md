---
name: release
description: Publish Cuprita's crates to crates.io (public, permanent) and install them on another computer. Use only when the owner asks to release or publish.
---
# Release to crates.io

Publishing is public and cannot be undone: a version stays (`cargo yank` only hides it from
new users) and its name stays taken. Only the owner runs the upload: it needs their token.

## Publish (owner, own terminal; never paste the token into the chat)
1. First time: token from https://crates.io/settings/tokens (scopes `publish-new`,
   `publish-update`; the account needs a verified email), then `cargo login`.
2. Bump `[workspace.package] version` and the two `version = "…"` of `cuprita-core` and
   `cuprita-render` in the root `Cargo.toml` to the same number; `./check.sh`; commit.
3. `cargo publish --workspace --dry-run` (builds each crate from its tarball, uploads
   nothing), then `cargo publish --workspace`. It publishes core, render, app in that order;
   the three must carry the same version. A published version can't be replaced: fix, bump.

## Install on another computer
`rustup update` (needs Rust ≥ `rust-version` in `Cargo.toml`), then
`cargo install --locked cuprita` (installs the `cuprita` binary; `--locked` builds with the
tested dependency versions). Update: the same command with `--force`. Needs a Vulkan or
OpenGL driver; on Linux Wayland or X11.

## Gotchas
- A dirty tree makes `cargo publish` refuse (`--allow-dirty` only for dry runs).
- Packages hold only their crate's folder: nothing outside it (no `../`) may be included.
- Private alternative that publishes nothing: clone from a private git remote and
  `cargo install --locked --path crates/app`.

# Contributing to pikr

pikr is pre-1.0 and the mode trait + key dispatch surface is still in motion.
Please open an issue before starting any non-trivial PR so the design can be
sanity-checked early.

## Development setup

```bash
git clone git@github.com:kryptic-sh/pikr.git
cd pikr
rustup toolchain install stable    # rust-toolchain.toml pins this for you
cargo test --workspace
```

## Workspace layout

- `apps/pikr` — the `pikr` binary (CLI, UI wiring, mode dispatch)
- `xtask` — packaging + release automation

Internal libraries (matcher, modes, picker state) live inside `apps/pikr/src/`
until a second consumer appears. Don't extract a `crates/` layer speculatively —
wait for the second consumer.

## MSRV policy

`rust-version` in `Cargo.toml` tracks current stable Rust. Floor, not ceiling —
bumps land freely when new features are useful. Any bump must be logged in
`CHANGELOG.md` under the version that introduces it.

## Pull requests

- Branch from `main`. One logical change per PR.
- Commits: [Conventional Commits](https://www.conventionalcommits.org/) format.
  `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`, `ci`, `build`.
  Scope optional.
- Run before pushing:
  - `cargo fmt`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
- Markdown changes: `prettier --write <file>`.

## Releases

Cutting a release is the **BCTP** flow: bump the patch in `Cargo.toml`,
regenerate `Cargo.lock`, update `CHANGELOG.md` (move `Unreleased` to a new
version section), commit `chore: bump version`, tag `vX.Y.Z`, push commit + tag.
The tag triggers `release.yml`.

Patch for bug fixes / docs; minor for additive public API; major for breaking
changes.

## Reporting bugs / requesting features

Open a GitHub issue. For security issues, see `SECURITY.md` — do not file public
issues.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).

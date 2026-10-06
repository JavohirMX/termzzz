# Release Routine

## Goal
Release `termzzz` as a public GitHub project with reproducible macOS/Linux artifacts and optional package-manager distributions.

## Pre-Release Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo test` passes
- [ ] `cargo clippy --all-features --workspace -- -D warnings` passes
- [ ] README, configuration examples, and changelog match the release
- [ ] Version and lockfile are updated
- [ ] Canonical GitHub owner and release settings are configured
- [ ] Upstream MIT notice and attribution remain intact

Two checks are run in CI but were missing from this list, and both have caught
real problems: `cargo doc` with `RUSTDOCFLAGS="-D warnings"`, and `nix fmt --
--fail-on-change` plus `nix build`.

## Release Process

### 1. Choose the canonical repository

Set the GitHub remote and package metadata only after the repository owner and name are selected. Set the repository variable `TERMZZZ_CANONICAL_REPOSITORY` to the exact `owner/repository` value before enabling release or crates.io workflows. Do not point new releases at the historical upstream repository by accident.

### 2. Update release metadata

- Increment the version in `Cargo.toml`.
- Update `CHANGELOG.md`.
- Regenerate `Cargo.lock` through Cargo.
- Confirm the binary is named `termzzz`.
- Confirm release archives contain `termzzz` and its checksum file.

### 3. Build and verify locally

```bash
cargo build --release
cargo test
cargo fmt --all -- --check
cargo clippy --all-features --workspace -- -D warnings
```

Run representative effects manually in a real terminal:

```bash
./target/release/termzzz matrix
./target/release/termzzz ink
./target/release/termzzz terrain
```

### 4. Publish the GitHub release

Dispatch **Release Assets** with the tag name. It validates that the supplied tag
matches the `Cargo.toml` version *and* points at the current `main` SHA, then
builds aarch64 and x86_64 macOS and x86_64 Linux archives with a `.sha256` beside
each. It must be dispatched from `main`, because the build job is gated on
`github.ref == 'refs/heads/main'`.

```bash
gh workflow run release.yml --ref main -f tag_name=v0.0.1
```

### 5. Publish to crates.io

**Only after the release assets are up**, so `cargo install termzzz` never
resolves to a version whose release page is empty. Publish locally first: a bad
token or a metadata problem is much easier to diagnose outside CI. Set the
`CRATES_IO_TOKEN` secret afterwards and let `publish.yml` handle later versions.

crates.io does not allow a version to be republished, so anything found after a
publish needs a new number.

### 6. Package-manager distribution

The Homebrew tap lives at **`github.com/JavohirMX/homebrew-termzzz`**, so the tap
name is `JavohirMX/termzzz`:

```bash
brew tap JavohirMX/termzzz
brew install JavohirMX/termzzz/termzzz
```

It is a **binary formula**, not a source build: it installs the archive the
release workflow already publishes and verifies it against the SHA-256 published
beside it. That keeps the formula from having to track the dependency tree, and
it means the checksums in the formula must be updated on every release.

**The top-level `url` must be the Linux one**, with `on_macos` overriding it. The
reverse does not work — `on_linux` may not carry a `url` or a `sha256` — and
`brew style` says so on the first attempt:

```
FormulaAudit/ComponentsOrder: on_linux cannot include url
```

Run `brew style Formula/termzzz.rb` before pushing. It reorders the component
block and catches the trailing-newline offence too.

The formula is written from scratch, with its own source, checksums, class and
test command, and is not copied from any upstream formula.

## Release Notes Template

```markdown
## Version X.Y.Z

### Added
- Feature or effect description

### Changed
- User-visible change

### Fixed
- Bug fix description

### Compatibility
- `termzzz` is a clean break from earlier package and command names.
```

## Settled Decisions

- **Canonical repository** — `github.com/JavohirMX/termzzz`. The repository
  variable `TERMZZZ_CANONICAL_REPOSITORY` is set to that exact value, and every
  release and publish workflow refuses to run without it matching
  `github.repository`.
- **Crates.io publication** — done at 0.0.1; the name was unclaimed.
- **Homebrew** — the tap is `JavohirMX/homebrew-termzzz`, so users run
  `brew tap JavohirMX/termzzz` then `brew install JavohirMX/termzzz/termzzz`.
  Binary formula, checksums taken from the release assets.
- **Repository description** — "23 terminal screensavers and generative visual
  effects, in Rust".

## Deferred Decisions

- New project tagline

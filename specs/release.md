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

The release workflow is manual until the canonical repository is configured. It validates that the supplied tag matches the `Cargo.toml` version, then builds macOS and Linux assets and attaches checksums. Publishing to crates.io remains a separate explicit decision because the new crate namespace and credentials must be configured first.

### 5. Package-manager distribution

Create a `termzzz` Homebrew formula only after the canonical repository and release asset URL are known. A formula should install the `termzzz` binary and test `termzzz --version`; it must be written from scratch rather than copied from an upstream formula, with its own source, checksum, class, and test command.

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

## Deferred Decisions

- Canonical GitHub owner and repository URL
- Crates.io publication
- Homebrew tap ownership and formula location
- New project tagline

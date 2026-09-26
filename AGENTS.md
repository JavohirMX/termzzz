# Project Overview

## Quick Reference

`termzzz` is a collection of terminal-based screensavers and generative visual effects written in Rust.

### Main Commands
```bash
# Build
cargo build --release

# Run effects
termzzz matrix      # Matrix digital rain
termzzz life        # Conway's Game of Life
termzzz maze        # Maze generation
termzzz boids       # Boids flocking simulation
termzzz cube        # 3D cube rotation
termzzz crab        # ASCII crab animation
termzzz donut       # 3D donut rotation
termzzz pipes       # Pipe maze animation
termzzz plasma      # Plasma effect
termzzz fire        # Fire simulation
termzzz terrain     # Terrain generation
termzzz constellation # Drifting stars and dotted connections
termzzz dvd         # Bouncing ASCII logo
termzzz blank       # Blank screen
termzzz ascii       # Interactive generative ASCII field

# Run a timed playlist of effects
termzzz --playlist matrix,dvd,plasma
termzzz --shuffle

# Development
cargo test        # Run tests
cargo bench       # Run benchmarks

# Code Quality
cargo fmt --check  # Check formatting
cargo test --lib   # Run library tests
cargo clippy       # Run linter
```

### Installation
- **Local checkout**: `cargo install --path .`
- **Crates.io**: `cargo install termzzz` after publication is enabled
- **GitHub**: canonical repository and release links are pending selection

## Project Status
- **Version**: 0.2.0
- **Effects**: 15 screensavers and visual effects, plus a playlist mode
- **Platforms**: macOS and Linux
- **Configuration**: `~/.config/termzzz.toml`

## Current Focus
Ship the `termzzz` 0.2.0 release: global speed control, the DVD logo, playlist mode, and the canonical GitHub and distribution metadata.

## Architecture
See `specs/overview.md` for the project architecture and technical overview.

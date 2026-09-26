# Project Overview

## Quick Reference

`termzzz` (Terminal Arts) - A collection of terminal-based screensavers written in Rust.

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

# Development
cargo test        # Run tests
cargo bench       # Run benchmarks

# Code Quality (run after changes)
cargo fmt --check  # Check formatting
cargo test --lib   # Run library tests
cargo clippy       # Run linter
```

### Installation Methods
- **Homebrew**: `brew install oiwn/tap/termzzz`
- **Cargo**: `cargo install termzzz`
- **Manual**: Download from GitHub releases

## Project Status
- **Version**: 0.1.23
- **Effects**: 12 working screensavers
- **Platforms**: macOS (x86_64, arm64), Linux
- **Homebrew**: Tap available and working

## Current Focus
Preparing for public release and Reddit announcement. See `specs/current_task.md` for detailed release preparation plan.

## Architecture
See `specs/overview.md` for detailed project architecture and technical overview.
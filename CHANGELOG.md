# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - Unreleased

### Added
- Context-driven input and frame state through the runtime module
- Interactive generative ASCII field effect available with `termzzz ascii`
- Pointer interaction, pause/resume, reseeding, palette cycling, and brush-size controls
- Resize-safe rendering with screen-derived option updates for existing effects
- `terrain` is now available through the normal CLI validator
- Bouncing ASCII logo effect available with `termzzz dvd`, with a configurable multi-line logo
- Timed playlist mode via `--playlist`, `--shuffle`, and `--transition`, also configurable in `[playlist]`
- Central effect registry so the CLI, help output, check mode, and playlists share one source of truth
- Global speed control through `[global] speed`, `--speed <MULT>`, and the `+`/`-` keys

### Changed
- `+` and `-` change global animation speed; the mouse wheel resizes the ASCII brush
- Conway's Game of Life advances at a configurable generations-per-second rate
- Matrix, cube, and DVD effects advance on real frame deltas rather than fixed step counts
- Calmer defaults for plasma, boids, donut, pipes, cube, crab, and life
- Check mode now honours the configured effect options and global speed
- Release and crates.io workflows now require explicit manual dispatch

### Compatibility
- `termzzz` is a clean break from earlier package and command names, and provides no compatibility aliases

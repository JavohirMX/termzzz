# termzzz

Terminal screensavers and generative visual effects written in Rust.

`termzzz` is a collection of memory-safe terminal effects: a set of classic screensavers plus a new interactive ASCII field, playlist mode, and global speed control.

## Effects

- Matrix rain
- Conway's Game of Life
- Maze generation
- Boids flocking
- 3D cube
- ASCII crab animation
- Rotating donut
- Pipes
- Plasma
- Fire
- Constellation
- Terrain
- Bouncing DVD logo (`dvd`)
- Blank screen
- Interactive generative ASCII field (`ascii`)

## Usage

```bash
termzzz matrix
termzzz life
termzzz maze
termzzz boids
termzzz cube
termzzz crab
termzzz donut
termzzz dvd
termzzz pipes
termzzz fire
termzzz plasma
termzzz constellation
termzzz terrain
termzzz blank
termzzz ascii
```

Press `q`, `Esc`, or `Ctrl+C` to exit. `+` and `-` change the global animation speed in every effect. In ASCII mode:

- `r` reseeds the field
- `Space` pauses and resumes the animation
- `[` and `]` cycle glyph palettes
- The mouse wheel resizes the pointer brush
- Pointer movement temporarily brightens the field, then fades out

### Speed

`--speed` scales every effect, on top of calmer per-effect defaults:

```bash
termzzz plasma --speed 0.5
```

### Playlist

Play effects back to back, wiping to blank between each one. `--playlist` takes a
comma-separated list, and `--shuffle` plays the configured playlist in a random
order (every effect runs once per round):

```bash
termzzz --playlist matrix,dvd,plasma
termzzz --shuffle
termzzz --playlist matrix,dvd --transition 1.5
```

With no `--playlist` list, `--shuffle` runs every registered effect. Playlists can
also be defined in the config file:

```toml
[playlist]
shuffle = true
transition = 0.6

[[playlist.effects]]
effect = "matrix"
duration = 12.0

[[playlist.effects]]
effect = "dvd"
```

`duration` is optional and defaults per effect. Leaving `effects` empty plays
every effect.

## Installation

For a local checkout, install it with:

```bash
cargo install --path .
```

Once the canonical package repository is configured and published, `cargo install termzzz` will be available.

The project currently targets macOS and Linux. Windows support can be added after the initial ASCII release.

## Development

```bash
cargo build --release
cargo test
cargo fmt --check
cargo clippy
```

Configuration is optional. Generate a default TOML file with:

```bash
termzzz --print-config > ~/.config/termzzz.toml
```

Global speed is set with `[global] speed`, and the DVD logo is configurable:

```toml
[global]
speed = 1.0

[dvd]
logo = "termzzz"
speed = 9.0
corner_color_change = true
```

Use `\n` inside `logo` for multi-line logos.

## Attribution

Built on MIT-licensed code from an earlier terminal screensaver project; the original copyright notice remains in `LICENSE`.

## License

MIT. See `LICENSE` for the original copyright and permission notice.

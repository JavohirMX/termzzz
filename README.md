# termzzz

Terminal screensavers and generative visual effects written in Rust.

`termzzz` is a collection of memory-safe terminal effects: a set of classic screensavers plus a new interactive ink field, playlist mode, and global speed control.

## Effects

- Matrix rain
- Conway's Game of Life
- Mandelbrot set (`mandelbrot`)
- Maze generation
- Boids flocking
- 3D cube
- ASCII crab animation
- Rotating donut
- Pipes
- Plasma
- Fire
- 3D solar system (`solarsystem`)
- Terrain
- Bouncing DVD logo (`dvd`)
- Blank screen
- Interactive generative field you pour ink into (`ink`)
- Several Langton's ants sharing one board (`ants`)
- First-person flight over a fractal height field (`flyover`)
- Slime-mould agents building a transport network (`physarum`)
- Interfering waves from a few point sources (`ripple`)
- Newton's method basins, coloured by which root each sample finds (`newton`)

## Usage

```bash
termzzz matrix
termzzz life
termzzz mandelbrot
termzzz maze
termzzz boids
termzzz cube
termzzz crab
termzzz donut
termzzz dvd
termzzz pipes
termzzz fire
termzzz plasma
termzzz solarsystem
termzzz terrain
termzzz ants
termzzz flyover
termzzz physarum
termzzz ripple
termzzz newton
termzzz blank
termzzz ink
```

Every effect that uses randomness is seeded, so a run can be reproduced or shared:

```bash
termzzz matrix --seed 1234
termzzz --playlist matrix,dvd,plasma --seed 1234
```

Leave `--seed` off and every effect draws a fresh seed at startup, so no two runs
look the same. Pin one with `--seed` to reproduce a particular run, or per effect in
the config file, for example `[matrix] seed = 7`.

Press `q`, `Esc`, or `Ctrl+C` to exit. `+` and `-` change the global animation speed
in every effect. `n` and `p` move to the next and previous effect, behind the same
diagonal wipe the playlist uses, so you can walk the whole catalogue without
restarting.

Click in boids mode to scatter the flock. A click drops a shockwave that travels
outward and pushes the boids it passes away from where you clicked; the ring is
drawn, so you can see where the wave is and how far it reaches. Drag to leave a
trail of them. Just moving the mouse does nothing — the flock only reacts to a
button press. The flock closes up again afterwards, since the rules that hold it
together are still running.

In ink mode:

- `r` reseeds the field
- `Space` pauses and resumes the animation
- `[` and `]` cycle glyph palettes
- The mouse wheel resizes the pointer brush
- Pointer movement temporarily brightens the field, then fades out

Mouse capture is enabled for the whole session whenever the running effect (or any
effect in the playlist) is one of the two that read it, so `n` can reach an
interactive effect at any moment without the terminal having to be reconfigured
mid-run.

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

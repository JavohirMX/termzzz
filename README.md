# termzzz

![matrix, dvd, solarsystem, ink, ripple, aquarium and clock](assets/montage.gif)

Terminal screensavers and generative visual effects, written in Rust.

`termzzz` is a catalogue of 23 effects that draw themselves in the terminal, from
the classics to generative pieces a line-drawing terminal can draw properly. It
runs one effect, plays a playlist, or walks the whole catalogue behind a wipe.

Everything is reproducible. Every effect that uses randomness is seeded, so any
run can be shared or replayed exactly.

## Install

```bash
cargo install termzzz
```

Or from a checkout:

```bash
cargo install --path .
```

macOS and Linux are the supported targets. The test suite also runs on Windows
in CI, but Windows is not a supported platform yet.

## Effects

| Effect | What you see |
|---|---|
| `matrix` | Digit rain, each drop fading through its own tail |
| `life` | Conway's Game of Life. The glyph is the cell's age, so generations are visible |
| `mandelbrot` | The escape-time Mandelbrot set, zooming and panning |
| `maze` | Maze generation, held when it finishes |
| `boids` | A flock. Click to scatter it; drag to leave a trail |
| `cube` | A rotating 3D cube, filled and shaded by depth |
| `crab` | Crabs scuttling along a sloping seabed |
| `donut` | A rotating 3D donut |
| `dvd` | The DVD wordmark, bouncing, in braille |
| `pipes` | Pipe maze growth |
| `plasma` | Interfering waves. The glyph is the value |
| `fire` | Fire, rising and cooling |
| `solarsystem` | An orrery with tilted orbits and real orbital periods |
| `ink` | A generative field you pour ink into. Interactive |
| `terrain` | A side-view landscape: two ridges, parallax, sky |
| `ants` | Several Langton's ants sharing one board |
| `flyover` | First-person flight over a fractal height field |
| `physarum` | Slime-mould agents building a transport network |
| `ripple` | Waves from a few point sources, interfering |
| `newton` | Newton's method on the complex plane, coloured by which root each cell reaches |
| `aquarium` | A fish tank. Seven species, two rendering media, shaded by depth |
| `clock` | A large clock, with a rule showing the seconds |
| `blank` | A blank screen. What a playlist rests on |

Some effects draw with braille cells (8 dots each) or half blocks (two colours
per cell), so the same terminal shows more detail than a character grid can.

## Run one

```bash
termzzz matrix
termzzz aquarium
termzzz flyover
```

`q`, `Esc`, or `Ctrl+C` exits. `+` and `-` change the global animation speed in
every effect. `n` and `p` move to the next and previous effect behind a diagonal
wipe, so you can walk the whole catalogue without restarting.

## Play a playlist

Play effects back to back, wiping to blank between each one:

```bash
termzzz --playlist matrix,dvd,plasma
termzzz --playlist matrix,dvd --transition 1.5
```

`--shuffle` plays every registered effect in a random order, each once per
round:

```bash
termzzz --shuffle
```

Set per-effect durations in the config file:

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

## Reproducible runs

Leave `--seed` off and every effect draws a fresh seed at startup, so no two runs
look the same. Pin one to reproduce a particular run:

```bash
termzzz matrix --seed 1234
termzzz --playlist matrix,dvd --seed 1234
```

You can also set the seed per effect in the config file, for example
`[matrix] seed = 7`.

## Mouse

Click in `boids` mode to scatter the flock. A click drops a shockwave that
travels outward and pushes the boids it passes away from where you clicked. The
ring is drawn, so you can see how far it reaches. Drag to leave a trail. Just
moving the mouse does nothing; the flock reacts to a button press and then
closes up again, because the rules that hold it together keep running.

In `ink` mode:

- `r` reseeds the field
- `Space` pauses and resumes
- `[` and `]` cycle glyph palettes
- the mouse wheel resizes the pointer brush
- pointer movement brightens the field, then the brightness fades

Mouse capture is enabled for the whole session whenever the running effect, or
any effect in the playlist, reads it. That way `n` can reach an interactive
effect at any moment without the terminal being reconfigured mid-run.

## Configuration

Configuration is optional. Generate a default file with:

```bash
termzzz --print-config > ~/.config/termzzz.toml
```

Global speed is `[global] speed`. The DVD logo is configurable, and `\n` inside
`logo` gives you multiple lines:

```toml
[global]
speed = 1.0

[dvd]
logo = "termzzz"
speed = 9.0
corner_color_change = true
```

When the terminal does not have focus, effects throttle to `[global] idle_fps`
and freeze rather than slow down, so nothing jumps when you come back.

## Development

```bash
cargo build --release
cargo test
cargo fmt --all -- --check
cargo clippy --all-features --workspace --all-targets -- -D warnings
cargo run --release --bin frame_times   # frame cost and ANSI volume per effect
```

## Attribution

Built on MIT-licensed code from [oiwn/tarts](https://github.com/oiwn/tarts), an
earlier terminal screensaver project. The original copyright notice remains in
`LICENSE`.

## License

MIT. See `LICENSE`.
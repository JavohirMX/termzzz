# Recording termzzz Effects as GIFs

## The pipeline that works

`asciinema` captures, `agg` renders. Both are in Homebrew:

```bash
brew install asciinema agg
```

The font matters more than anything else here. The crate draws braille
(U+2800-28FF) and the block elements (U+2580-259F), and **most monospace fonts
carry only one of the two**. Measured across 555 installed fonts: Menlo, Andale
Mono, DejaVu Sans Mono, Fira Code and JetBrains Mono all have the block elements
and **no braille at all**. DejaVu Sans has all 256 braille patterns and is
proportional, which destroys the grid. Iosevka Term has both and is monospace:

```bash
# Iosevka is the font to use. Verify before recording, not after.
curl -sL -o iosevka.zip \
  https://github.com/be5invis/Iosevka/releases/download/v34.9.0/PkgTTC-Iosevka-34.9.0.zip
unzip -q iosevka.zip -d iose && cp iose/Iosevka-Regular.ttc ~/Library/Fonts/
```

Verify coverage with `fontTools`, checking the cmap rather than trusting the
font's reputation:

```python
from fontTools.ttLib import TTFont
cmap = set(TTFont('Iosevka-Regular.ttc', fontNumber=0).getBestCmap())
assert all(c in cmap for c in range(0x2800, 0x2900))   # braille
assert all(c in cmap for c in range(0x2581, 0x2589))   # lower blocks
```

## Two things that will cost you an hour

**A backgrounded effect has no controlling terminal, and crossterm needs one.**
`termzzz` fails immediately with `Failed to initialize input reader` if it cannot
open the tty. `crossterm::tty_fd()` uses stdin when stdin is a tty and otherwise
opens `/dev/tty`, so the effect must run in the **foreground** of a session that
has a controlling terminal. Signaling it means sending `SIGINT` from a separate
background process, not `&`-ing the effect itself:

```bash
#!/bin/bash
TZ=/path/to/target/release/termzzz
for spec in matrix:2.5 blank:0.35 dvd:4.0; do
  name="${spec%%:*}"; secs="${spec##*:}"
  ( sleep "$secs"; pkill -INT -f "release/termzzz $name" ) &
  "$TZ" "$name" --seed 1234
  wait
done
```

Run that under `script` so the session has a pty:

```bash
asciinema rec --overwrite --window-size 120x30 \
  --command "script -q /dev/null bash /tmp/cap/run.sh" out.cast
```

**`--playlist` on the command line discards config durations.** `main.rs` only
starts a playlist when `--playlist` or `--shuffle` is given, and passing
`--playlist` replaces `PlaylistOptions::effects` with names that carry their own
`default_duration` — 15s for `matrix`, 30s for `aquarium`. A playlist defined in
`~/.config/termzzz.toml` with explicit `duration` values is ignored in favour of
the defaults, and the run is a single effect if neither flag is passed. **For a
montage with per-effect timing, drive the loop in the shell as above.**

**`agg` does not clear the screen when the alternate screen is entered.** Running
each effect as its own process gives a clean terminal, but the *replay* keeps
whatever the previous effect painted, because `?1049h` is not treated as a clear.
`ripple` paints the whole screen `rgb(0,70,143)` and the clock paints nothing at
all, so the clock arrives on ripple's blue — which reads as a missing clear and
looks like a bug in `termzzz`. It is not one; a real terminal clears on `?1049h`.
Insert the clear the terminal would have done:

```python
import json
lines = [l for l in open('cast') if l.strip()]
hdr, ev = lines[0], [json.loads(l) for l in lines[1:]]
for e in ev:
    if e[1] == 'o' and '\x1b[?1049h' in e[2]:
        e[2] = e[2].replace('\x1b[?1049h', '\x1b[?1049h\x1b[2J\x1b[0m')
with open('cast_clear', 'w') as f:
    f.write(hdr)
    for e in ev: f.write(json.dumps(e) + '\n')
```

To find out *which* effect owns a colour rather than guessing, split the stream on
`\x1b[?1049h` and count background SGR codes per segment. That is how the blue was
attributed to `ripple` rather than to the effect that appeared to be showing it.

## Rendering

```bash
agg --font-family "Iosevka Term" --font-size 16 \
    --font-dir ~/Library/Fonts --fps-cap 9 out.cast assets/montage.gif
```

`agg` takes the output path as a positional argument. There is no `-o`. The
`--fps-cap` is the main size lever: 120x30 at 9 fps over 24 seconds is about
2.8 MB.

## Choosing what goes in

**Check that every effect actually appears before you ship the file.** Render a
contact sheet and look at it:

```bash
ffmpeg -i montage.gif \
  -vf "select='eq(n\,10)+eq(n\,32)+eq(n\,60)+eq(n\,85)',scale=400:-1,tile=2x2" \
  -frames:v 1 sheet.png
```

Frame numbers are not time, so guess the sample points from the first pass and
adjust. Two effects were cut from the montage on the evidence:

- `physarum` needs about 2,500 steps to grow its network, which is roughly 40
  seconds at 60 fps. Any short slot shows only the five seed blobs, so it reads
  as an unfinished picture rather than a transport network.
- `dvd` and `solarsystem` move slowly — `dvd` crosses a wall every 5.7 to 8.3
  seconds at 80x24 — so they need about 4 seconds each or they look frozen.

## `vhs` is not usable here

`vhs` requires both terminal dimensions to be at least 120 **cells**, so a
realistic 120x30 capture is impossible. 120 rows is also not an honest size for
these effects, which are tuned for 24 to 50 rows.
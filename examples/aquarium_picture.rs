// Throwaway: what does the tank actually look like?
//
// The frame table cannot tell you whether an effect reads, and neither can a
// symbol grid on its own -- the depth cue lives entirely in the background
// channel, which a grid of characters shows nothing of. So this prints three
// things: the glyphs, a letter per *distinct* background colour so the gradient
// is visible, and the fish colours by depth.
//
// Diffs are accumulated the way a terminal accumulates them, because that is
// what the picture on screen is.
//
// `--art` prints something different and more basic: each species' dot grid, one
// character per braille dot, at both poses. The rendered tank is a braille glyph
// per cell, so a cell-grid dump of a fish is a handful of unreadable `⠿⠛⠉`
// glyphs -- the art cannot be judged from the picture the effect draws. It has to
// be judged in the space it is authored in.

use termzzz::aquarium::art;
use termzzz::aquarium::{Aquarium, AquariumOptions};
use termzzz::common::TerminalEffect;
use termzzz::render::palette::perceptual_distance;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--art") => {
            print_art();
            return;
        }
        Some("--fish") => {
            print_fish();
            return;
        }
        _ => {}
    }

    let (w, h) = (100usize, 30usize);
    let frames: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(240);

    let mut tank = Aquarium::new(
        AquariumOptions {
            seed: 7,
            ..AquariumOptions::default()
        },
        (w as u16, h as u16),
    );

    // Accumulate changed cells, exactly as the encoder would.
    let mut grid = vec![(' ', termzzz::buffer::Cell::default()); w * h];
    for _ in 0..frames {
        tank.advance_for_picture(1.0 / 60.0);
        for (x, y, cell) in tank.get_diff() {
            grid[y * w + x] = (cell.symbol, cell);
        }
    }

    println!("== glyphs ==\n");
    for y in 0..h {
        let line: String = (0..w).map(|x| grid[y * w + x].0).collect();
        println!("|{line}|");
    }

    // One letter per distinct background colour, in the order first seen. The
    // water is a static gradient, so most of the frame is a small number of
    // bands.
    println!("\n== backgrounds, one letter per distinct colour ==");
    let mut letters = std::collections::HashMap::new();
    let mut next = 0usize;
    let alphabet: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@%#*+=-.!?/\\|~^:;,".chars().collect();
    for (_, cell) in &grid {
        let key = format!("{:?}", cell.bg);
        if !letters.contains_key(&key) {
            letters.insert(key.clone(), alphabet[next % alphabet.len()]);
            next += 1;
        }
    }
    for y in 0..h {
        let line: String = (0..w)
            .map(|x| {
                *letters
                    .get(&format!("{:?}", grid[y * w + x].1.bg))
                    .unwrap_or(&' ')
            })
            .collect();
        println!("|{line}|");
    }
    // Distinct colours is NOT the bandwidth number. What costs escape sequences
    // is a cell whose background differs from the last one written, so that is
    // what gets counted: runs of equal background along a row.
    let mut changes = 0usize;
    for y in 0..h {
        for x in 1..w {
            if grid[y * w + x].1.bg != grid[y * w + x - 1].1.bg {
                changes += 1;
            }
        }
    }
    println!("\ndistinct backgrounds: {}", letters.len());
    println!(
        "bg changes between horizontally adjacent cells: {changes} of {} ({:.1}%)",
        (h * (w - 1)),
        100.0 * changes as f64 / (h * (w - 1)) as f64
    );
    let mut keys: Vec<_> = letters.iter().collect();
    keys.sort_by_key(|(_, c)| **c);
    for (k, c) in keys {
        println!("  {c}  {k}");
    }

    println!("\n== foregrounds in use (fish colours) ==");
    let mut fg = std::collections::BTreeMap::new();
    for (_, cell) in &grid {
        if cell.symbol != ' ' {
            *fg.entry(format!("{:?}", cell.color)).or_insert(0usize) += 1;
        }
    }
    let rows: Vec<(String, usize)> = fg.into_iter().collect();
    for (k, n) in &rows {
        println!("  {n:5}  {k}");
    }
    if rows.len() >= 2 {
        // Are the fish colours actually separated, or all one hue?
        let parse = |s: &String| match s.as_str() {
            "Rgb { r: 0, g: 0, b: 0 }" => termzzz::buffer::Cell::default().color,
            _ => {
                // crude parse of "Rgb { r: R, g: G, b: B }"
                let nums: Vec<u8> = s
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|t| !t.is_empty())
                    .map(|t| t.parse().unwrap_or(0))
                    .collect();
                if nums.len() >= 3 {
                    crossterm::style::Color::Rgb {
                        r: nums[0],
                        g: nums[1],
                        b: nums[2],
                    }
                } else {
                    termzzz::buffer::Cell::default().color
                }
            }
        };
        let mut worst = f32::INFINITY;
        for i in 0..rows.len() {
            for j in (i + 1)..rows.len() {
                worst = worst
                    .min(perceptual_distance(parse(&rows[i].0), parse(&rows[j].0)));
            }
        }
        println!(
            "\nclosest pair of fish colours: {worst:.3} OKLab (0.02 = just noticeable)"
        );
    }
}

/// Each species, as a dot grid, at every pose.
///
/// Three columns per pose: the dots, the shading as a digit, and the braille
/// glyphs the terminal will actually show. The third is here because the first
/// two are the authored space and the third is the result, and comparing them is
/// the only way to see a mistake in the dot-to-cell resampling.
fn print_art() {
    for species in art::sources() {
        // The character species have no dot grid and no girth, and the two media
        // are not the same kind of thing: one is a procedural shape, one is a
        // drawing. Printed separately rather than in a table of Nones.
        if let art::Source::Chars(sp) = species {
            let a = termzzz::aquarium::charart::species(sp);
            println!(
                "== {} : {} columns by {} rows, {} frames, cruise {}, depth {} ==",
                sp.name,
                a.cells_wide(),
                a.cells_tall(),
                termzzz::aquarium::charart::POSES,
                sp.cruise,
                sp.depth
            );
            for pose in 0..termzzz::aquarium::charart::POSES {
                println!("-- frame {pose}");
                for y in 0..a.cells_tall() {
                    let row: String = (0..a.cells_wide())
                        .map(|x| {
                            a.cells(pose)
                                .find(|(cx, cy, _, _)| {
                                    *cx == x as i32 && *cy == y as i32
                                })
                                .map(|(_, _, ch, _)| ch)
                                .unwrap_or(' ')
                        })
                        .collect();
                    println!("  |{row}|");
                }
            }
            continue;
        }
        let art::Source::Dots(species) = species else {
            unreachable!("handled above")
        };
        println!(
            "== {} : {} dots long, girth {} ==",
            species.name, species.length, species.girth
        );
        for pose in 0..art::POSES {
            let fish = art::rasterise(species, pose);
            println!("-- pose {pose} ({}x{} dots)", fish.width(), fish.height());

            if std::env::args().any(|a| a == "--rows") {
                // Dots, part and shade on one line per row. Three separate blocks
                // are unreadable: aligning a dot grid against a tone map by eye is
                // how you spend an afternoon concluding that two arrays disagree
                // when they do not and you counted wrong.
                for y in 0..fish.height() {
                    let w = fish.width();
                    let dots: String = (0..w)
                        .map(|x| if fish.dot(x, y) { '#' } else { '.' })
                        .collect();
                    let part: String =
                        (0..w).map(|x| fish.part_at(x, y).tag()).collect();
                    let shade: String = (0..w)
                        .map(|x| {
                            if !fish.dot(x, y) {
                                ' '
                            } else {
                                char::from_digit(
                                    (fish.shade_at(x, y) * 9.0).round() as u32,
                                    10,
                                )
                                .unwrap_or('?')
                            }
                        })
                        .collect();
                    println!("   {y:2} |{dots}| |{part}| |{shade}|");
                }
                continue;
            }

            let (w, h) = (fish.width(), fish.height());
            println!("   dots  ");
            for y in 0..h {
                let line: String = (0..w)
                    .map(|x| if fish.dot(x, y) { '#' } else { '.' })
                    .collect();
                println!("   |{line}|");
            }
            println!("   shade ");
            for y in 0..h {
                let line: String = (0..w)
                    .map(|x| {
                        let s = fish.shade_at(x, y);
                        if !fish.dot(x, y) {
                            ' '
                        } else {
                            char::from_digit(
                                (s * 9.0).round().clamp(0.0, 9.0) as u32,
                                10,
                            )
                            .unwrap_or('.')
                        }
                    })
                    .collect();
                println!("   |{line}|");
            }
            // Which dot is *what*. The art's own tests need this -- an eye and an
            // anal fin are both very dark and a symbol grid cannot tell them
            // apart -- and so does anyone trying to work out why a fish looks
            // wrong.
            println!("   part  ");
            for y in 0..h {
                let line: String =
                    (0..w).map(|x| fish.part_at(x, y).tag()).collect();
                println!("   |{line}|");
            }
            println!("   glyph ");
            for cy in 0..fish.cells_tall() {
                let line: String = (0..fish.cells_wide())
                    .map(|cx| fish.cell_char(cx, cy))
                    .collect();
                println!("   |{line}|");
            }

            // The ink's own bounding box, because the bitmap is padded to hold
            // the tallest fin and a fish is not the size of its bitmap. Reporting
            // the bitmap's aspect says a neon is 2:1 when the animal in it is 3:1.
            let (mut x0, mut x1, mut y0, mut y1) = (w, 0usize, h, 0usize);
            let mut ink = 0usize;
            for y in 0..h {
                for x in 0..w {
                    if fish.dot(x, y) {
                        ink += 1;
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
            let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
            println!(
                "   ink {ink} dots, {bw}x{bh} extent = {:.2}:1 w:h   (bitmap {w}x{h})",
                bw as f32 / bh as f32
            );
        }
        println!();
    }
}

/// Where the fish are, what the mix is, and whether any of it is on screen.
///
/// Three questions a glyph dump answers badly. A picture of two overlapping piles
/// looks like a bug in the renderer and is actually a bug in the shoal: twenty-five
/// fish of fifteen cells cannot resolve in a hundred-column tank, so the
/// separation pass jams them into a corner -- and the first person to look at
/// that spent a while blaming braille. The per-row histogram is what gives the
/// jam away, and the drawn-cell count is what gives away the opposite failure,
/// where `art::FishArt::cells` was yielding dot offsets as if they were cell
/// offsets and every fish landed off the bottom of the screen.
fn print_fish() {
    let (w, h) = (100usize, 30usize);
    let mut tank = Aquarium::new(
        AquariumOptions {
            seed: 7,
            ..AquariumOptions::default()
        },
        (w as u16, h as u16),
    );
    for _ in 0..240 {
        tank.advance_for_picture(1.0 / 60.0);
    }
    let fish = tank.fish_snapshot();
    println!("{} fish in {w}x{h}", fish.len());
    // The species names, in index order, from the art tables' union. `SPECIES`
    // alone is only the two braille species now that the tank has two media, and
    // indexing a species id against it puts the third fish in a five-fish tank out
    // of bounds -- which is this example's whole job noticing.
    let names: Vec<&str> = art::sources().map(|s| s.name()).collect();
    let mut by_species = std::collections::BTreeMap::new();
    let mut rows = vec![0usize; h];
    for (i, (x, y, z, s, ..)) in fish.iter().enumerate() {
        *by_species.entry(names[*s]).or_insert(0usize) += 1;
        rows[(y.round().max(0.0) as usize).min(h - 1)] += 1;
        println!("  {i:3} {:<7} x={x:6.1} y={y:5.1} z={z:.2}", names[*s]);
    }
    println!("  mix: {by_species:?}");
    println!("  fish per row: {rows:?}");

    // Is any of it on the terminal? The water is in the background channel, so
    // this is the only number here that says whether the frame is doing anything.
    let diff = tank.get_diff();
    let glyphs = diff.iter().filter(|(_, _, c)| c.symbol != ' ').count();
    println!(
        "  last frame: {} cells changed, {glyphs} with a glyph",
        diff.len()
    );
    if glyphs == 0 {
        println!("  NOTHING IS DRAWN. The tank is water and gravel.");
    }

    // And what the art table holds, for cross-checking against the sprite dump.
    println!("\n  species:");
    for sp in art::sources() {
        match sp {
            art::Source::Dots(d) => {
                let drawn = art::rasterise(d, 0);
                let cells: Vec<_> = drawn.cells().collect();
                println!(
                    "    {:<7} {:>3} dots long, girth {:>4.1} -> {:>2}x{:<2} cells, {:>3} drawn  [braille, cruise {}, depth {}]",
                    d.name,
                    d.length,
                    d.girth,
                    drawn.cells_wide(),
                    drawn.cells_tall(),
                    cells.len(),
                    d.cruise,
                    d.depth
                );
            }
            art::Source::Chars(c) => {
                let a = termzzz::aquarium::charart::species(c);
                let drawn: Vec<_> = a.cells(0).collect();
                println!(
                    "    {:<7} {:>18} -> {:>2}x{:<2} cells, {:>3} drawn  [chars,   cruise {}, depth {}]",
                    c.name,
                    "hand-drawn",
                    a.cells_wide(),
                    a.cells_tall(),
                    drawn.len(),
                    c.cruise,
                    c.depth
                );
            }
        }
    }
}

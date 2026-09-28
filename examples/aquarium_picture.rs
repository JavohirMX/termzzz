// Throwaway: what does the tank actually look like?
//
// The frame table cannot tell you whether an effect reads, and neither can a
// symbol grid on its own -- the depth cue lives entirely in the background
// channel, which a grid of characters shows nothing of. So this prints three
// things: the glyphs, a letter per *distinct* background colour so the gradient
// and the shafts are visible, and the fish colours by depth.
//
// Diffs are accumulated the way a terminal accumulates them, because that is
// what the picture on screen is.

use termzzz::aquarium::{Aquarium, AquariumOptions};
use termzzz::common::TerminalEffect;
use termzzz::render::palette::perceptual_distance;

fn main() {
    let (w, h) = (100usize, 30usize);
    let frames: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(240);

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
    // bands; the shafts are the rest.
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

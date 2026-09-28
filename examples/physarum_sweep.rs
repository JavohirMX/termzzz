// Throwaway: what do the numbers actually read at the shipped settings, on a few
// board sizes, and at the bad sensor distance? Used to set the test thresholds
// from measurements instead of from a guess.
use std::collections::VecDeque;

use termzzz::physarum::{Physarum, PhysarumOptions};

struct Shape {
    marked: usize,
    components: usize,
    largest: usize,
    edge_ratio: f32,
}

fn shape(trail: &[f32], width: usize, height: usize, floor: f32) -> Shape {
    let above = |i: usize| trail[i] > floor;
    let mut seen = vec![false; width * height];
    let (mut marked, mut components, mut largest, mut best_ratio) =
        (0usize, 0usize, 0usize, 0.0f32);
    for start in 0..width * height {
        if seen[start] || !above(start) {
            continue;
        }
        components += 1;
        let mut cells = Vec::new();
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(i) = queue.pop_front() {
            marked += 1;
            cells.push(i);
            let (x, y) = (i % width, i / width);
            for (nx, ny) in [
                ((x + width - 1) % width, y),
                ((x + 1) % width, y),
                (x, (y + height - 1) % height),
                (x, (y + 1) % height),
            ] {
                let n = ny * width + nx;
                if !seen[n] && above(n) {
                    seen[n] = true;
                    queue.push_back(n);
                }
            }
        }
        if cells.len() > largest {
            largest = cells.len();
            let mut boundary = 0usize;
            for i in &cells {
                let (x, y) = (i % width, i / width);
                for (nx, ny) in [
                    ((x + width - 1) % width, y),
                    ((x + 1) % width, y),
                    (x, (y + height - 1) % height),
                    (x, (y + 1) % height),
                ] {
                    if !above(ny * width + nx) {
                        boundary += 1;
                    }
                }
            }
            best_ratio = boundary as f32 / cells.len() as f32;
        }
    }
    Shape {
        marked,
        components,
        largest,
        edge_ratio: best_ratio,
    }
}

fn main() {
    let base = PhysarumOptions::default();

    for (w, h) in [(60u16, 20u16), (120, 40), (200, 60), (400, 200)] {
        let (fw, fh) = (w as f32 * 2.0, h as f32);
        println!("\n=== {w}x{h} cells, {} field rows ===", fw * fh);
        for (label, distance) in [("shipped d=9", 9.0f32), ("bad d=1.5", 1.5)] {
            for seed in [3u64, 5, 9] {
                let mut p = Physarum::new(
                    PhysarumOptions {
                        agents: ((fw * fh * PHYSARUM) as u32).max(40),
                        sensor_distance: distance,
                        ..base.clone()
                    },
                    (w, h),
                );

                for _ in 0..4_000 {
                    p.step();
                }
                let (pw, ph) = p.field_dimensions();
                let s = shape(p.trail_view(), pw, ph, 3.0);
                let share = s.largest as f32 / s.marked.max(1) as f32;
                let coverage = s.marked as f32 / (pw * ph) as f32;
                println!(
                    "  {label:<11} seed {seed}: marked {:>5.2}%  \
                     parts {:>4}  largest {:>5} ({share:.2} of marked)  \
                     edge/area {:.2}",
                    coverage * 100.0,
                    s.components,
                    s.largest,
                    s.edge_ratio
                );
            }
        }
    }
}

const PHYSARUM: f32 = 0.002;

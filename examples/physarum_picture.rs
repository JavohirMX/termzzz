// Throwaway: print the field as a picture for the candidate parameter sets,
// because a connectivity number cannot tell you whether the thing reads.
use termzzz::physarum::{Physarum, PhysarumOptions};

const RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '%', '#', '@'];

fn render(physarum: &Physarum, cols: usize) {
    let (fw, fh) = physarum.field_dimensions();
    let trail = physarum.trail_view();
    let peak = trail.iter().copied().fold(0.0f32, f32::max).max(1.0);
    // The field is twice as tall as it is wide in cells, and a cell is about twice
    // as tall as it is wide, so the printed shape wants about cols/2 rows.
    let rows = (cols / 2).clamp(4, fh);
    let bx = fw as f32 / cols as f32;
    let by = fh as f32 / rows as f32;

    for r in 0..rows {
        let mut line = String::new();
        for c in 0..cols {
            let mut sum = 0.0f32;
            let mut n = 0.0f32;
            let x0 = (c as f32 * bx) as usize;
            let y0 = (r as f32 * by) as usize;
            for y in y0..((y0 as f32 + by).ceil() as usize).min(fh) {
                for x in x0..((x0 as f32 + bx).ceil() as usize).min(fw) {
                    sum += trail[y * fw + x];
                    n += 1.0;
                }
            }
            let t = if n > 0.0 { sum / n / peak } else { 0.0 };
            let index = ((t.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32).round()
                as usize)
                .min(RAMP.len() - 1);
            line.push(RAMP[index]);
        }
        println!("{line}");
    }
}

fn main() {
    let (w, h) = (120u16, 44u16);
    let field_rows = w as f32 * h as f32 * 2.0;
    let base = PhysarumOptions::default();

    let candidates: [(&str, PhysarumOptions); 3] = [
        (
            "share 0.001 d 9 spread 0.1 decay 0.90",
            PhysarumOptions {
                agents: (field_rows * 0.001) as u32,
                sensor_distance: 9.0,
                spread: 0.1,
                decay: 0.9,
                ..base.clone()
            },
        ),
        (
            "share 0.002 d 9 spread 0.1 decay 0.90",
            PhysarumOptions {
                agents: (field_rows * 0.002) as u32,
                sensor_distance: 9.0,
                spread: 0.1,
                decay: 0.9,
                ..base.clone()
            },
        ),
        (
            "share 0.005 d 9 spread 0.1 decay 0.94",
            PhysarumOptions {
                agents: (field_rows * 0.005) as u32,
                sensor_distance: 9.0,
                spread: 0.1,
                decay: 0.94,
                ..base.clone()
            },
        ),
    ];

    for (label, options) in candidates {
        println!("\n=== {label} ===");
        for steps in [1_000usize, 4_000, 12_000] {
            let mut physarum = Physarum::new(options.clone(), (w, h));
            for _ in 0..steps {
                physarum.step();
            }
            println!("--- {steps} steps ---");
            render(&physarum, 120);
        }
    }
}

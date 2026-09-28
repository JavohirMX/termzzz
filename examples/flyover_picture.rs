// Throwaway: what does one column's march actually produce? Prints the projected
// row at each step for a few columns, so the projection can be checked against
// hand arithmetic rather than guessed at.
#![allow(dead_code)]

use termzzz::common::TerminalEffect;
use termzzz::flyover::{Flyover, FlyoverOptions};

fn main() {
    let mut flyover = Flyover::new(
        FlyoverOptions {
            seed: 1,
            ..FlyoverOptions::default()
        },
        (100u16, 32u16),
    );
    for _ in 0..120 {
        flyover.advance(1.0 / 60.0);
    }
    flyover.get_diff();

    let (dw, dh) = flyover.dot_dimensions();
    println!("dot grid {dw}x{dh} (DOTS_Y=4)");
    println!(
        "horizon row {:.2}, centre row {:.1}, FOV_TAN is private",
        flyover.horizon_row_value(),
        dh as f32 * 0.5
    );
    println!(
        "camera y {:.2} z {:.2} x {:.2}, pitch {:.3}, roll {:.3}",
        flyover.camera_height(),
        flyover.camera_depth(),
        flyover.camera_lateral(),
        flyover.pitch_value(),
        flyover.roll_value()
    );
    let ground_under = flyover.ground_under_camera();
    println!("ground directly under the camera: {ground_under:.2}");

    // And what the frame holds.
    let (inked, blank, top) = flyover.coverage_summary();
    println!("\ninked {inked}, blank {blank}, topmost empty row {top}");
    for y in 0..flyover.dimensions().1 {
        let mut line = String::new();
        for x in 0..flyover.dimensions().0 {
            line.push(flyover.cell_symbol(x, y));
        }
        println!("{line}");
    }
}

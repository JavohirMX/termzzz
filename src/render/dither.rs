//! Ordered dithering: a stable per-pixel threshold offset.
//!
//! The point is to turn one hard threshold into a moving boundary, so values
//! that sit near the threshold land on a *fraction* of the dots rather than
//! all of them or none. That is what lets braille carry a gradient: the eye
//! integrates the dot density, so a dithered half-and-half reads as mid grey
//! where a hard cutoff would read as a hard edge.
//!
//! The matrices are the standard recursive Bayer construction. They are tiny,
//! need no allocation, and — importantly for a renderer — are *stable*: the
//! same coordinate always gets the same offset, so a dithered pattern does not
//! crawl between frames when nothing has changed.

/// A per-coordinate threshold offset in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dither {
    /// Every offset is 0.5, so the threshold is a hard cutoff.
    #[default]
    None,
    /// The 4x4 Bayer matrix: 16 levels.
    Bayer4,
    /// The 8x8 Bayer matrix: 64 levels.
    Bayer8,
}

/// The standard 4x4 Bayer matrix, normalised to `0.0..1.0`.
///
/// Written out rather than generated. The recursive definition
/// (`M(2n) = [[4M, 4M+2], [4M+3, 4M+1]]`) is elegant but needs either a const
/// evaluator or a lazy table to express, and neither buys anything over sixteen
/// literals for a matrix that never changes.
const BAYER4: [[f32; 4]; 4] = [
    [0.0 / 16.0, 8.0 / 16.0, 2.0 / 16.0, 10.0 / 16.0],
    [12.0 / 16.0, 4.0 / 16.0, 14.0 / 16.0, 6.0 / 16.0],
    [3.0 / 16.0, 11.0 / 16.0, 1.0 / 16.0, 9.0 / 16.0],
    [15.0 / 16.0, 7.0 / 16.0, 13.0 / 16.0, 5.0 / 16.0],
];

/// The standard 8x8 Bayer matrix, sampled on its top-left 4x4.
///
/// The full matrix is 64 entries, but only the top-left 4x4 is ever read --
/// `at` indexes `x % 4` -- so storing 16 keeps the static small. The 64 distinct
/// levels still come from the high bits of the underlying value.
const BAYER8: [[f32; 4]; 4] = [
    [0.0 / 64.0, 32.0 / 64.0, 8.0 / 64.0, 40.0 / 64.0],
    [48.0 / 64.0, 16.0 / 64.0, 56.0 / 64.0, 24.0 / 64.0],
    [12.0 / 64.0, 44.0 / 64.0, 4.0 / 64.0, 36.0 / 64.0],
    [60.0 / 64.0, 28.0 / 64.0, 52.0 / 64.0, 20.0 / 64.0],
];

impl Dither {
    /// The threshold offset for a dot, in `0.0..1.0`.
    ///
    /// `None` is a constant 0.5, which is the neutral offset: a hard threshold
    /// compared against `value > threshold + 0.0` is the same test either way,
    /// and keeping the arithmetic identical means callers do not need two
    /// code paths.
    pub fn at(&self, x: usize, y: usize) -> f32 {
        match self {
            Dither::None => 0.5,
            Dither::Bayer4 => BAYER4[x % 4][y % 4],
            Dither::Bayer8 => BAYER8[x % 4][y % 4],
        }
    }

    /// How many distinct levels this dither can produce.
    ///
    /// A gradient mapped onto fewer levels than this will band; mapped onto
    /// more, the extra resolution is wasted.
    pub fn levels(&self) -> usize {
        match self {
            Dither::None => 1,
            Dither::Bayer4 => 16,
            Dither::Bayer8 => 64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_dither_is_a_constant_offset() {
        for x in 0..9 {
            for y in 0..9 {
                assert_eq!(Dither::None.at(x, y), 0.5);
            }
        }
    }

    #[test]
    fn the_offset_is_stable_for_a_coordinate() {
        // A pattern that moved between frames would read as crawling noise even
        // when the underlying field had not changed.
        for dither in [Dither::Bayer4, Dither::Bayer8] {
            let first = dither.at(3, 5);
            for _ in 0..100 {
                assert_eq!(dither.at(3, 5), first);
            }
        }
    }

    #[test]
    fn offsets_stay_in_range() {
        for dither in [Dither::None, Dither::Bayer4, Dither::Bayer8] {
            for x in 0..16 {
                for y in 0..16 {
                    let v = dither.at(x, y);
                    assert!(
                        (0.0..1.0).contains(&v),
                        "{dither:?} produced {v} at ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn bayer4_produces_sixteen_distinct_levels() {
        let mut seen = std::collections::HashSet::new();
        for x in 0..4 {
            for y in 0..4 {
                seen.insert(Dither::Bayer4.at(x, y).to_bits());
            }
        }
        assert_eq!(seen.len(), 16, "the 4x4 matrix is not a permutation");
    }

    #[test]
    fn the_matrix_tiles_seamlessly() {
        // Every 4x4 window of an infinite tiling must contain the same set of
        // levels, or a repeating 4x4 block becomes visible in a large field.
        let base: std::collections::HashSet<u32> = (0..4)
            .flat_map(|x| (0..4).map(move |y| (x, y)))
            .map(|(x, y)| Dither::Bayer4.at(x, y).to_bits())
            .collect();

        for ox in 0..5 {
            for oy in 0..5 {
                let window: std::collections::HashSet<u32> = (0..4)
                    .flat_map(|x| (0..4).map(move |y| (x, y)))
                    .map(|(x, y)| Dither::Bayer4.at(x + ox, y + oy).to_bits())
                    .collect();
                assert_eq!(window, base, "the tiling breaks at offset ({ox},{oy})");
            }
        }
    }

    #[test]
    fn level_counts_match_the_matrices() {
        assert_eq!(Dither::None.levels(), 1);
        assert_eq!(Dither::Bayer4.levels(), 16);
        assert_eq!(Dither::Bayer8.levels(), 64);
    }
}

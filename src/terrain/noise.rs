// Base permutation table for Perlin noise.
static BASE: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36,
    103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75,
    0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149,
    56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27,
    166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41,
    55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100,
    109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118,
    126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42,
    223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155,
    167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178,
    185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191,
    179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199,
    106, 157, 184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205,
    93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];

/// Doubled so the wrap-around lookup never needs a modulo.
#[derive(Clone)]
struct Permutation([u8; 512]);

impl Permutation {
    /// Shuffles the base table deterministically from `seed`.
    ///
    /// The table used to be a single shared constant, so `PerlinNoise::new`
    /// stored the seed and then ignored it: every terrain, at every seed,
    /// rendered the identical landscape, while the config file advertised
    /// `seed` as if it did something.
    fn seeded(seed: u64) -> Self {
        let mut p = [0u8; 256];
        p.copy_from_slice(&BASE);

        // Fisher-Yates driven by a small deterministic generator, so the same
        // seed always shuffles the same way without pulling in a dependency.
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };

        for i in (1..256).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }

        let mut doubled = [0u8; 512];
        doubled[..256].copy_from_slice(&p);
        doubled[256..].copy_from_slice(&p);
        Self(doubled)
    }
}

/// Perlin noise over a seeded permutation table.
///
/// The table is boxed rather than held inline. It is 512 bytes, and inlining it
/// made every `Terrain` 584 bytes, which in turn made the `AnyEffect` enum in
/// the registry 584 bytes because of a single oversized variant. The table is
/// built once per terrain, so the allocation costs nothing per frame.
pub struct PerlinNoise {
    permutation: Box<Permutation>,
}

impl PerlinNoise {
    pub fn new(seed: u64) -> Self {
        Self {
            permutation: Box::new(Permutation::seeded(seed)),
        }
    }

    pub fn noise_2d(&self, x: f64, y: f64) -> f64 {
        // Find unit square containing point
        let xi = (x.floor() as i32) & 255;
        let yi = (y.floor() as i32) & 255;

        // Find relative position in square
        let xf = x - x.floor();
        let yf = y - y.floor();

        // Compute fade curves
        let u = fade(xf);
        let v = fade(yf);

        // Hash coordinates of square corners. The table is doubled, so these
        // wrap with a mask rather than a division.
        let p = &self.permutation.0;
        let xi = xi as usize & 255;
        let yi = yi as usize & 255;
        let xi1 = (xi + 1) & 255;

        let aa = p[(p[xi] as usize + yi) & 511] as usize;
        let ab = p[(p[xi] as usize + yi + 1) & 511] as usize;
        let ba = p[(p[xi1] as usize + yi) & 511] as usize;
        let bb = p[(p[xi1] as usize + yi + 1) & 511] as usize;

        // Interpolate between gradients
        let x1 = lerp(grad_2d(aa, xf, yf), grad_2d(ba, xf - 1.0, yf), u);
        let x2 = lerp(
            grad_2d(ab, xf, yf - 1.0),
            grad_2d(bb, xf - 1.0, yf - 1.0),
            u,
        );

        lerp(x1, x2, v)
    }

    pub fn octave_noise_2d(
        &self,
        x: f64,
        y: f64,
        octaves: i32,
        persistence: f64,
        scale: f64,
    ) -> f64 {
        // `octaves` is a plain integer in the config file, so zero and negative
        // values are reachable. Looping zero times leaves `max_value` at 0.0 and
        // the division produced NaN, which propagated through the visualisation
        // match and rendered the whole screen as a single flat glyph.
        let octaves = octaves.clamp(1, 16);

        let mut total = 0.0;
        let mut frequency = if scale > 0.0 { scale } else { f64::EPSILON };
        let mut amplitude = 1.0;
        let mut max_value = 0.0;

        for _ in 0..octaves {
            total += self.noise_2d(x * frequency, y * frequency) * amplitude;
            max_value += amplitude;
            amplitude *= persistence;
            frequency *= 2.0;
        }

        if max_value == 0.0 || !total.is_finite() {
            return 0.0;
        }

        let normalised = total / max_value;
        if normalised.is_finite() {
            normalised.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

fn grad_2d(hash: usize, x: f64, y: f64) -> f64 {
    let h = hash & 3;
    match h {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_noise() {
        let a = PerlinNoise::new(1234);
        let b = PerlinNoise::new(1234);
        for i in 0..64 {
            let (x, y) = (i as f64 * 0.37, i as f64 * 0.11);
            assert_eq!(a.noise_2d(x, y), b.noise_2d(x, y), "at {i}");
        }
    }

    #[test]
    fn different_seeds_give_different_noise() {
        // The seed used to be stored and ignored, so every seed produced the
        // identical landscape.
        let a = PerlinNoise::new(1);
        let b = PerlinNoise::new(2);

        let differs = (0..64).any(|i| {
            let (x, y) = (i as f64 * 0.37, i as f64 * 0.11);
            (a.noise_2d(x, y) - b.noise_2d(x, y)).abs() > f64::EPSILON
        });

        assert!(differs, "two seeds produced identical noise");
    }

    #[test]
    fn the_permutation_is_a_valid_shuffle() {
        // Every value appears exactly once in the first half, and the second
        // half mirrors it.
        let p = Permutation::seeded(7).0;
        let mut seen = [false; 256];
        for &value in &p[..256] {
            assert!(!seen[value as usize], "value {value} appeared twice");
            seen[value as usize] = true;
        }
        assert!(seen.iter().all(|s| *s), "some values were missing");
        assert_eq!(&p[..256], &p[256..], "the table was not doubled");
    }

    #[test]
    fn different_seeds_shuffle_differently() {
        assert_ne!(Permutation::seeded(1).0, Permutation::seeded(2).0);
    }

    #[test]
    fn the_same_seed_shuffles_identically() {
        assert_eq!(Permutation::seeded(99).0, Permutation::seeded(99).0);
    }

    #[test]
    fn noise_stays_inside_its_range() {
        let noise = PerlinNoise::new(3);
        for i in 0..256 {
            let value = noise.noise_2d(i as f64 * 0.13, i as f64 * 0.29);
            assert!(
                (-1.0..=1.0).contains(&value),
                "noise_2d returned {value} at {i}"
            );
        }
    }

    #[test]
    fn octaves_sum_within_range() {
        let noise = PerlinNoise::new(5);
        for i in 0..32 {
            let value = noise.octave_noise_2d(
                i as f64 * 0.21,
                i as f64 * 0.17,
                4,
                0.5,
                0.02,
            );
            assert!(value.is_finite(), "octave_noise_2d returned {value}");
        }
    }

    #[test]
    fn zero_octaves_does_not_divide_by_zero() {
        // `octaves` is a user-facing integer in the config file, and the
        // normalisation divides by the sum of the amplitudes.
        let noise = PerlinNoise::new(5);
        let value = noise.octave_noise_2d(1.5, 2.5, 0, 0.5, 0.02);
        assert!(value.is_finite(), "zero octaves produced {value}");
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn the_noise_generator_stays_small() {
        // The permutation table is 512 bytes. Held inline it made every Terrain
        // 584 bytes, and therefore the whole AnyEffect enum, because of one
        // oversized variant.
        assert!(
            size_of::<PerlinNoise>() <= 16,
            "PerlinNoise grew to {} bytes",
            size_of::<PerlinNoise>()
        );
    }
}

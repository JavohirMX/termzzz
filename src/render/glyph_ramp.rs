//! A ramp of characters ordered by how much ink they put on the page.
//!
//! Most effects in this crate want the same thing: turn a scalar into a
//! character, where the character gets denser as the scalar rises. Each of them
//! grew its own version, and each got it slightly wrong in a different way --
//! one indexed a ramp by an absolute value so the pale end grew with the window,
//! one cast a float to `u8` and saturated a quarter of the screen onto index 0,
//! one quantised the glyph and the colour independently and ended up with bands
//! that disagreed. This is the one version, with the mistakes written down.
//!
//! # The ordering is the whole point
//!
//! A ramp has to run from sparse to dense, and it is easy to get backwards
//! without noticing, because a wrong ramp still looks like *something*. The donut
//! recorded the failure in its own source: a previous palette put near-white
//! cream on the `.` and `,` that cover most of the torus, so the majority of the
//! screen was the brightest thing on it.
//!
//! Ink coverage cannot be measured here, so this module does not pretend to.
//! What it does instead:
//!
//! - ships presets whose ordering is deliberate, with the reasoning recorded
//!   next to each one,
//! - treats a user-configured ramp as the user's decision, in the order they
//!   wrote it, and
//! - documents the requirement at every call site that has a reason to care.
//!
//! # Width
//!
//! Characters are filtered only for being control codes. Some are
//! double-width, and a double-width character in a cell-indexed grid shears
//! rather than merely looking wrong -- see [`is_ambiguous_or_narrow`], which is
//! the predicate for the subset that is safe in any terminal.

//! Named character sets, for discoverability.
//!
//! Eight effects in this crate draw from a ramp, and the useful sets were
//! previously either duplicated or buried in whichever effect happened to need
//! one first. Listed here so `glyphs = "blocks"` in a config file means
//! something findable, and so `AGENTS.md` has one place to point at.
pub mod presets {
    /// The classic ten, and the default everywhere.
    ///
    /// Note that it is *not* monotonic in ink: `=` is heavier than `+`, and `+` is
    /// heavier than `*`. Three consecutive steps read backwards. It is tolerable
    /// as texture and wrong as a value carrier, which is why [`BLOCKS`] exists.
    pub const SHADE: &str = " .:-=+*#%@";

    /// Five steps, very high contrast, for a small window or a coarse field.
    ///
    /// The gaps are large on purpose. At 40x12 a ten-step ramp spends most of
    /// its entries on differences the eye cannot resolve across a cell.
    pub const SPARSE: &str = " .:oO@";

    /// Dot-led, for a field that should read as points rather than as shading.
    pub const DOTS: &str = " .·:•+";

    /// The four Unicode shade blocks, plus a space and a light dot.
    ///
    /// `░` `▒` `▓` `█` are *defined* as quarter, half, three-quarter and full
    /// coverage of the cell box, so this is the only ramp here whose ordering is
    /// guaranteed by the standard rather than by taste. It also holds up at any
    /// cell aspect ratio, which a punctuation ramp does not: a `:` reads as a
    /// horizontal smear in a wide cell and a vertical dotted line in a tall one.
    ///
    /// The blocks are single-width and East_Asian_Width = Neutral, so unlike
    /// [`DOTS`] this is safe in a CJK-configured terminal.
    pub const BLOCKS: &str = " ░▒▓█";

    /// The eight lower-eighth blocks, plus a space: a smooth vertical gradient.
    ///
    /// `▁` through `▇` are the *eighth* blocks, at U+2581 to U+2587, and they are
    /// a finer and more even instrument than [`BLOCKS`]: eight steps instead of
    /// three, each a one-eighth increment of the cell's height, and unlike the
    /// shade blocks they are anchored to the *bottom* of the cell. That anchoring
    /// is the point for a flame or a waveform, where each row is a sample and the
    /// row's height is the value -- [`BLOCKS`] would draw the same eight values
    /// in an order that has nothing to do with which way up they are.
    ///
    /// The ink coverage is not monotonic in a subtle way worth stating: `▁` is a
    /// baseline rule and `█` is the full box, and the seven between them climb in
    /// one-eighth steps of the *same* bar, so unlike [`SHADE`] there is no point
    /// at which the ramp reads backwards. Eight steps is also the reason this
    /// exists next to `BLOCKS`: an eight-bit intensity field rendered through a
    /// five-entry ramp shows five bands, and through this one it does not.
    ///
    /// Font risk is low but not zero, and it is *local*: `█` and the eighth
    /// blocks are in the Block Elements range that every font with box drawing
    /// has, and a terminal missing the lot falls back to a replacement glyph
    /// rather than a blank. The set is single-width and East_Asian_Width =
    /// Ambiguous, the same class as `░▒▓█` above and for the same reason
    /// [`is_ambiguous_or_narrow`] allows it.
    pub const FLAME: &str = " ▁▂▃▄▅▆▇█";

    /// [`SHADE`] with its leading space removed, for a *filled region* whose
    /// lowest value is still part of the picture.
    ///
    /// Every other set here starts with a space, and for most effects that is
    /// right: "no value" should be "no ink". But a space is a hole in a filled
    /// region, and this module's docs carry the rule -- the sparsest step lands on
    /// the part of the region with the lowest value, and a space there makes that
    /// part invisible against whatever is behind it.
    ///
    /// `newton` is the caller, and it is the worst possible case. Its regions are
    /// Newton's basins, coloured by which root a sample converged to, and the
    /// lowest ink step covers the samples that converged *fastest* -- which are
    /// the ones nearest a root, and so the largest smooth area of each basin. A
    /// space there would delete the middle of every basin and leave only the
    /// fractal filigree, which is the part of the picture meant to be the detail.
    ///
    /// The ordering is [`SHADE`]'s, including its not-quite-monotonic stretch in
    /// the middle. That is tolerable as texture and wrong as a value carrier, which
    /// is what [`BLOCKS`] exists for -- but this set's job is to be *present*
    /// everywhere, and five uneven steps the eye can rank beat four steps of
    /// nothing at the bottom.
    pub const INKED: &str = ".:-=+*#%@";

    /// Every set above, for `--help` and the docs.
    pub const ALL: &[(&str, &str)] = &[
        ("shade", SHADE),
        ("sparse", SPARSE),
        ("dots", DOTS),
        ("blocks", BLOCKS),
        ("flame", FLAME),
        ("inked", INKED),
    ];

    /// Looks a set up by name, case-insensitively.
    pub fn by_name(name: &str) -> Option<&'static str> {
        ALL.iter()
            .find(|(preset, _)| preset.eq_ignore_ascii_case(name))
            .map(|(_, glyphs)| *glyphs)
    }
}

/// Characters that will not reliably occupy exactly one cell.
///
/// Returns false for genuinely double-width characters, which shear a grid that
/// is indexed by cell. Returns true for *ambiguous* width -- `·`, `•`, the
/// Latin-1 supplement -- which is one column in a Latin-configured terminal and
/// two in a CJK-configured one.
///
/// A real width table wants `unic-width`, which is a dependency this crate does
/// not have for the sake of one assertion. The ranges below are the ones that
/// actually occur in ramps like these.
///
/// The Block Elements range U+2581 to U+2588 -- `▁▂▃▄▅▆▇█`, the shade blocks and
/// the eighth blocks -- is here as one span because the arm used to single out
/// `█` alone, which allowed the top of [`presets::BLOCKS`] and rejected the seven
/// characters directly below it. That was an oversight in the list rather than a
/// decision about them: all eight are East_Asian_Width = Ambiguous, which is the
/// class this function already accepts, and `no_preset_contains_a_character_that
/// _would_shear_a_grid` is what notices when a new preset reaches past the end of
/// a range.
pub fn is_ambiguous_or_narrow(glyph: char) -> bool {
    glyph.is_ascii()
        || ('\u{00A0}'..='\u{00FF}').contains(&glyph)
        || ('\u{2000}'..='\u{206F}').contains(&glyph)
        || ('\u{2190}'..='\u{2BFF}').contains(&glyph)
        || ('\u{FF61}'..='\u{FF9F}').contains(&glyph)
        || ('\u{2581}'..='\u{2588}').contains(&glyph)
        || ('\u{2591}'..='\u{2593}').contains(&glyph)
        || glyph == '\u{25A0}'
}

/// A never-empty ramp of characters, sparse to dense.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphRamp {
    glyphs: Vec<char>,
}

/// The fallback, used whenever a configured ramp turns out to be unusable.
///
/// ASCII on purpose: it is the one set guaranteed single-width in every terminal
/// ever shipped, so a bad config degrades to "less interesting" rather than "a
/// sheared grid".
const FALLBACK: &str = presets::SHADE;

impl GlyphRamp {
    /// Builds a ramp from characters, dropping control codes.
    ///
    /// Never empty. An empty ramp used to underflow `len() - 1` and panic in the
    /// donut, and that was reachable straight from a user config file, so
    /// emptiness is handled here rather than at each call site.
    pub fn new(glyphs: Vec<char>) -> Self {
        let glyphs: Vec<char> = glyphs
            .into_iter()
            .filter(|glyph| !glyph.is_control())
            .collect();
        if glyphs.is_empty() {
            return Self::new_ascii(FALLBACK.chars().collect());
        }
        Self { glyphs }
    }

    /// Builds a ramp from a string, dropping control codes.
    ///
    /// Named `from_text` rather than `from_str` because it cannot fail: there is
    /// no error to return, and an infallible `from_str` reads like a fallible one
    /// that forgot to report.
    pub fn from_text(glyphs: &str) -> Self {
        Self::new(glyphs.chars().collect())
    }

    /// Builds a ramp from a string, dropping anything that is not ASCII as well.
    ///
    /// For effects whose glyphs are indexed against a character *count* in a
    /// config file, where a silently dropped multi-byte character would make the
    /// configured set and the drawn set disagree.
    pub fn new_ascii(glyphs: Vec<char>) -> Self {
        let glyphs: Vec<char> = glyphs
            .into_iter()
            .filter(|glyph| glyph.is_ascii() && !glyph.is_control())
            .collect();
        if glyphs.is_empty() {
            return Self {
                glyphs: FALLBACK.chars().collect(),
            };
        }
        Self { glyphs }
    }

    /// Number of steps. Always at least one.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// Always false. A [`GlyphRamp`] is never empty, which is the point of it.
    ///
    /// Present so `len() == 0` and `is_empty()` cannot drift apart at a call
    /// site that only has one of them.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The ramp index for a value in `0.0..=1.0`.
    ///
    /// Clamped, not wrapped. This is a brightness ramp, so a value above the top
    /// is the brightest entry and a value below the bottom is the sparsest --
    /// there is no meaningful "past the end". (A *cyclic* ramp wants
    /// [`index_wrapped`](Self::index_wrapped) instead, which is a different
    /// question and conflating the two is how a palette ends up saturating.)
    ///
    /// NaN is treated as the bottom, for the reason recorded in
    /// [`Palette::sample`](super::Palette::sample): `f32::clamp` propagates NaN,
    /// so a NaN would sail through this and index unpredictably.
    pub fn index_for(&self, value: f32) -> usize {
        let value = if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        };
        if self.glyphs.len() == 1 {
            return 0;
        }
        ((value * (self.glyphs.len() - 1) as f32).round() as usize)
            .min(self.glyphs.len() - 1)
    }

    /// The ramp index for a value in `0.0..=1.0`, wrapping at the top.
    ///
    /// For a cyclic ramp -- a colour wheel, a repeating pattern -- where going
    /// past the end should return to the beginning rather than saturate.
    pub fn index_wrapped(&self, value: f32) -> usize {
        let value = if value.is_nan() { 0.0 } else { value };
        if self.glyphs.len() == 1 {
            return 0;
        }
        let scaled = (value * self.glyphs.len() as f32).floor() as isize;
        scaled.rem_euclid(self.glyphs.len() as isize) as usize
    }

    /// The character at a ramp index. Clamped.
    pub fn at(&self, index: usize) -> char {
        self.glyphs[index.min(self.glyphs.len() - 1)]
    }

    /// The character for a value in `0.0..=1.0`.
    pub fn sample(&self, value: f32) -> char {
        self.at(self.index_for(value))
    }

    /// The underlying characters, sparse to dense.
    pub fn glyphs(&self) -> &[char] {
        &self.glyphs
    }
}

impl Default for GlyphRamp {
    fn default() -> Self {
        Self::new_ascii(FALLBACK.chars().collect())
    }
}

impl FromIterator<char> for GlyphRamp {
    fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two ends have to be reachable, and so has everything between.
    #[test]
    fn every_step_of_the_value_range_is_reachable() {
        let ramp = GlyphRamp::from_text(presets::SHADE);
        let glyphs = ramp.glyphs();
        assert_eq!(glyphs.len(), 10);

        assert_eq!(ramp.sample(0.0), glyphs[0]);
        assert_eq!(ramp.sample(1.0), glyphs[9]);
        for (index, glyph) in glyphs.iter().enumerate() {
            let value = index as f32 / (glyphs.len() - 1) as f32;
            assert_eq!(ramp.sample(value), *glyph, "step {index} is unreachable");
        }
    }

    /// Past the top is the top. This is the mistake worth pinning: an earlier
    /// mandelbrot used an absolute escape count as the index, so five iterations
    /// already reached the end of the palette and everything above it was one
    /// flat colour.
    #[test]
    fn values_past_the_top_saturate_rather_than_wrapping() {
        let ramp = GlyphRamp::from_text(presets::SHADE);
        let top = ramp.sample(1.0);
        for value in [1.0, 1.5, 12.0, 96.0, 1.0e9, f32::INFINITY] {
            assert_eq!(ramp.sample(value), top, "{value} did not saturate");
        }
    }

    /// A cyclic ramp is a different question and has to be asked separately.
    #[test]
    fn the_wrapping_index_returns_to_the_start() {
        let ramp = GlyphRamp::from_text(presets::SHADE);
        assert_eq!(ramp.index_wrapped(0.0), 0);
        assert_eq!(ramp.index_wrapped(0.099), 0);
        assert_eq!(ramp.index_wrapped(0.1), 1);
        assert_eq!(ramp.index_wrapped(1.0), 0, "a full lap should land home");
        assert_eq!(ramp.index_wrapped(1.5), 5);
        assert_eq!(
            ramp.index_wrapped(-0.5),
            5,
            "negative should wrap backwards"
        );
    }

    /// An empty or all-control ramp must not panic.
    ///
    /// Reachable straight from a user config file, and the donut used to
    /// underflow `len() - 1` here.
    #[test]
    fn an_unusable_ramp_falls_back_rather_than_panicking() {
        for configured in [vec![], vec!['\n'; 4], vec!['\t', '\u{7}']] {
            let ramp = GlyphRamp::new(configured);
            assert!(!ramp.is_empty());
            assert_eq!(ramp.len(), 10, "the fallback is the default ramp");
            assert_eq!(ramp.sample(0.5), '+', "the fallback is not in order");
        }
        assert_eq!(
            GlyphRamp::new_ascii(vec!['░', '█']).len(),
            10,
            "new_ascii must fall back too, or it has the same panic"
        );
    }

    /// A single-character ramp is a degenerate but legal configuration.
    #[test]
    fn a_one_character_ramp_always_draws_that_character() {
        let ramp = GlyphRamp::from_text("#");
        assert_eq!(ramp.len(), 1);
        for value in [-1.0, 0.0, 0.5, 1.0, 99.0] {
            assert_eq!(ramp.sample(value), '#');
            assert_eq!(ramp.index_wrapped(value), 0);
        }
    }

    /// A NaN must not index unpredictably.
    #[test]
    fn a_nan_value_reads_as_the_sparsest_step() {
        let ramp = GlyphRamp::from_text(presets::SHADE);
        assert_eq!(ramp.sample(f32::NAN), ramp.sample(0.0));
        assert_eq!(ramp.index_for(f32::NAN), 0);
    }

    /// A huge index is the top, not a panic.
    #[test]
    fn an_out_of_range_index_is_clamped() {
        let ramp = GlyphRamp::from_text(presets::SHADE);
        let top = ramp.sample(1.0);
        for index in [9, 10, 1_000, usize::MAX] {
            assert_eq!(ramp.at(index), top, "index {index}");
        }
    }

    /// Every preset has to be usable and findable by its own name.
    #[test]
    fn every_preset_is_well_formed() {
        for (name, glyphs) in presets::ALL {
            let ramp = GlyphRamp::from_text(glyphs);
            assert!(!ramp.is_empty(), "preset {name} is empty");
            assert_eq!(
                ramp.len(),
                glyphs.chars().count(),
                "preset {name} lost characters"
            );
            assert_eq!(ramp.sample(0.0), glyphs.chars().next().unwrap());
            assert_eq!(
                ramp.sample(1.0),
                glyphs.chars().last().unwrap(),
                "preset {name} does not reach its own last character"
            );
            assert_eq!(presets::by_name(name), Some(*glyphs));
            assert_eq!(presets::by_name(&name.to_uppercase()), Some(*glyphs));
        }
        assert_eq!(presets::by_name("nonesuch"), None);
    }

    /// The presets have to be distinct, or listing them is pointless.
    #[test]
    fn the_presets_are_distinct_from_each_other() {
        for (index, (name, glyphs)) in presets::ALL.iter().enumerate() {
            for (other_name, other) in &presets::ALL[index + 1..] {
                assert_ne!(
                    glyphs, other,
                    "{name} and {other_name} are the same set"
                );
            }
        }
    }

    /// The blocks preset is the one whose ordering is guaranteed, and the reason
    /// it exists is that a punctuation ramp's ordering is a matter of taste that
    /// can be got backwards. The shade preset is documented as *not* monotonic,
    /// so this asserts the documented state of both rather than a rule that only
    /// one of them satisfies.
    #[test]
    fn the_ink_ordering_of_each_preset_is_as_documented() {
        // `░▒▓█` are defined as 1/4, 1/2, 3/4 and full coverage, so this holds
        // because of the standard. The midpoints of a five-step ramp land
        // exactly on each interior entry.
        let blocks = GlyphRamp::from_text(presets::BLOCKS);
        assert_eq!(blocks.sample(0.0), ' ');
        assert_eq!(blocks.sample(0.25), '░');
        assert_eq!(blocks.sample(0.5), '▒');
        assert_eq!(blocks.sample(0.75), '▓');
        assert_eq!(blocks.sample(1.0), '█');

        // The shade ramp is documented as non-monotonic: `=` is heavier than
        // `+`, which is heavier than `*`, so three consecutive steps read
        // backwards. If that ever stops being true the documentation is wrong,
        // and this is what catches it.
        let shade = GlyphRamp::from_text(presets::SHADE);
        assert_eq!(shade.sample(0.45), '=');
        assert_eq!(shade.sample(0.55), '+');
        assert_eq!(shade.sample(0.65), '*');
    }

    /// Nothing in a default preset may be double-width.
    #[test]
    fn no_preset_contains_a_character_that_would_shear_a_grid() {
        for (name, glyphs) in presets::ALL {
            for glyph in glyphs.chars() {
                assert!(
                    is_ambiguous_or_narrow(glyph),
                    "preset {name} contains {glyph:?} (U+{:04X})",
                    glyph as u32
                );
            }
        }
    }
}

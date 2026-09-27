use super::renderer::{
    AsciiRenderer, DEFAULT_PALETTE_NAME, GlyphPalette, PHOSPHOR_RAMP,
};
use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crate::runtime::{FrameContext, InputEvent, Key, KeyPhase, PointerPhase};
use crossterm::style::Color;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsciiFieldOptions {
    pub seed: u64,
    pub time_scale: f32,
    pub pointer_strength: f32,
    pub brush_radius: f32,
    pub glyphs: String,
    /// The colour stops the field's value is painted with, dimmest first.
    ///
    /// The user asked for the effect to be "fully white or a single color", and
    /// a list is what says both: one entry is a monochrome field, several are a
    /// ramp, and the default is the phosphor green with a neutral white top --
    /// see [`PHOSPHOR_RAMP`].
    ///
    /// Written in crossterm's own colour spellings, which are *not* the ones
    /// `Color::from_str` takes: `"dark_grey"` with an underscore, and
    /// `"rgb_(r,g,b)"` rather than `"rgb(r,g,b)"`. `"#rrggbb"`, the plain names
    /// and `"reset"` are the same either way. A list of one is the monochrome
    /// case, and `--print-config` writes it back as `["rgb_(255, 255, 255)"]`.
    ///
    /// An **empty** list is not a monochrome field: it falls back to
    /// [`PHOSPHOR_RAMP`], because the alternative is a screensaver that paints
    /// nothing and gives no reason. Anything the terminal can draw is kept
    /// rather than second-guessed, so `["reset"]` is a real answer -- "whatever
    /// my terminal's own foreground is" -- and `["ansi(15)"]` is too.
    ///
    /// # Interaction with `palette`
    ///
    /// **`palette` wins if both are set, and the default is set.** So a stock
    /// config -- which has `colors` written out in it, because
    /// `palette = "green"` is the default and resolving the default here would
    /// mean a generated config could not be told to use orange -- has to
    /// continue to work, and it can only do that if an untouched `colors` gets
    /// out of the way.
    ///
    /// That is the whole argument, and it has a cost: the two options are
    /// opposites rather than layers, so `palette = "orange"` alongside a
    /// configured `colors` list silently discards the list. Stated here rather
    /// than discovered later because there is no warning to give -- printing
    /// every time both are set would be noise on a stock config, where both
    /// *are* set by definition.
    ///
    /// `palette` is the friendlier spelling and it wins because it is the one
    /// that says what it means. To use the list, set `palette = ""`.
    pub colors: Vec<Color>,
    /// The name of a built-in colour ramp: `green`, `orange`, `blue`,
    /// `magenta`, `ice`, or `amber`.
    ///
    /// The user asked for this: "can we make the colors more varied? Currently
    /// it's green; can we have different settings, like an orange palette?"
    /// `colors` was already a knob and was the wrong shape for the request --
    /// getting orange out of it means writing `rgb_(255, 140, 32)` -- so the
    /// names are the answer and `colors` stays for everything the names do not
    /// cover.
    ///
    /// Case-insensitive. **An unknown name is not an error and not a blank
    /// screen**: it falls back to `green`, the same way an empty `colors` list
    /// falls back to [`PHOSPHOR_RAMP`], and for the same reason. A typo in a
    /// hand-edited config should give you the default, not an invisible effect.
    ///
    /// The empty string is the default, which means "use `colors`", which is
    /// what makes [`Self::colors`] still work -- see the note there on which of
    /// the two wins.
    ///
    /// `Palette::named` already resolves names against a *different* table, of
    /// multi-hue ramps that cycle. This one is deliberately a separate table of
    /// single-hue brightness ramps: this effect indexes a ramp by value and stops
    /// at the top, so it wants the other kind, and a user who typed
    /// `palette = "ember"` here would get a ramp that ends on amber for a field
    /// that is supposed to end on white. See [`super::FIELD_PALETTES`].
    pub palette: String,
}

impl Default for AsciiFieldOptions {
    /// Hand-written for the same reason as the plasma options': the real defaults
    /// have to live in one place, and a derived `Default` is how a config file
    /// that omits a key ends up with a zero in it.
    fn default() -> Self {
        Self {
            seed: 42,
            time_scale: 1.0,
            pointer_strength: 0.7,
            brush_radius: 0.2,
            glyphs: super::renderer::DEFAULT_GLYPHS.to_string(),
            // The default `colors` is still written, not left empty, for the
            // reason given on `colors`: `--print-config` writes every key, so a
            // generated config pins whatever the defaults were the day it was
            // generated, and a default that is "empty, meaning green" would make
            // every existing config resolve to green by accident rather than by
            // decision. It also means this field renders identically with or
            // without the new `palette` key, which is what "nothing changes for
            // anyone who has not asked for this" has to mean in practice.
            colors: PHOSPHOR_RAMP.to_vec(),
            palette: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PointerEnergy {
    position: (f32, f32),
    strength: f32,
    radius: f32,
}

pub struct AsciiField {
    screen_size: (u16, u16),
    options: AsciiFieldOptions,
    canvas: Canvas,
    values: Vec<f32>,
    renderer: AsciiRenderer,
    phase: f32,
    seed_state: u64,
    pub(crate) paused: bool,
    pointer: Option<PointerEnergy>,
    palette_index: usize,
}

impl TerminalEffect for AsciiField {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear();
        self.render_frame();
        self.canvas.commit()
    }

    fn update(&mut self) {
        if !self.paused {
            self.phase += 0.035 * self.options.time_scale;
        }
        self.decay_pointer();
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        if !self.paused {
            self.phase += context.delta.as_secs_f32() * self.options.time_scale;
        }
        self.decay_pointer();
    }

    fn handle_input(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Key { key, phase } if *phase == KeyPhase::Pressed => {
                self.handle_key(*key);
            }
            InputEvent::Key { .. } => {}
            InputEvent::Pointer {
                position,
                phase,
                button: _,
            } => self.handle_pointer(*position, *phase),
            InputEvent::Resize { .. } => {}
            InputEvent::FocusGained | InputEvent::FocusLost => {}
            InputEvent::Quit | InputEvent::Ignored => {}
        }
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        let width = self.screen_size.0 as usize;
        let height = self.screen_size.1 as usize;
        self.values = vec![0.0; width * height];
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.pointer = None;
    }

    fn reset(&mut self) {
        let paused = self.paused;
        let seed_state = self.seed_state;
        let phase = self.phase;
        let palette_index = self.palette_index;
        let mut reset = Self::new(self.options.clone(), self.screen_size);
        reset.paused = paused;
        reset.seed_state = seed_state;
        reset.phase = phase;
        reset.palette_index = palette_index;
        reset.set_palette();
        *self = reset;
    }
}

impl AsciiField {
    pub fn new(options: AsciiFieldOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let width = screen_size.0 as usize;
        let height = screen_size.1 as usize;
        let renderer = AsciiRenderer::new(GlyphPalette::new(
            &options.glyphs,
            resolve_colors(&options),
        ));
        let phase = Self::phase_for_seed(options.seed);

        Self {
            screen_size,
            values: vec![0.0; width * height],
            canvas: Canvas::new(screen_size.0, screen_size.1),
            renderer,
            phase,
            seed_state: options.seed,
            paused: false,
            pointer: None,
            palette_index: 0,
            options,
        }
    }

    fn phase_for_seed(seed: u64) -> f32 {
        (seed as f32 * 0.618_034) % TAU
    }

    fn render_frame(&mut self) {
        let width = self.screen_size.0 as usize;
        let height = self.screen_size.1 as usize;
        let aspect = width as f32 / height.max(1) as f32;
        let time = self.phase;
        let pointer = self.pointer;

        for y in 0..height {
            let v = y as f32 / height.max(1) as f32;
            for x in 0..width {
                let u = x as f32 / width.max(1) as f32;
                let warped_u = u + 0.12 * (v * 4.0 + time).sin();
                let warped_v = v + 0.12 * (u * 3.0 - time * 0.7).cos();
                let wave = ((warped_u * 3.2 + time).sin()
                    + (warped_v * 2.4 - time * 0.8).cos()
                    + ((warped_u + warped_v) * 4.6 + time * 0.35).sin())
                    / 3.0;
                let diagonal = (u * aspect * 1.7 - v * 2.2 + time * 0.5).sin();
                let mut value = 0.5 + 0.34 * wave + 0.16 * diagonal;

                if let Some(pointer) = pointer {
                    let dx = u - pointer.position.0;
                    let dy = (v - pointer.position.1) * aspect.max(0.5);
                    let distance = (dx * dx + dy * dy).sqrt();
                    let influence =
                        (1.0 - distance / pointer.radius.max(0.01)).max(0.0);
                    value += influence * influence * pointer.strength;
                }

                let index = y * width + x;
                self.values[index] = value.clamp(0.0, 1.0);
            }
        }

        self.renderer.render_field(
            &self.values,
            width,
            height,
            self.canvas.surface_mut(),
        );
    }

    fn handle_key(&mut self, key: Key) {
        match key {
            Key::Char('r') => {
                self.seed_state = self.seed_state.wrapping_add(1);
                self.phase = Self::phase_for_seed(self.seed_state);
                self.pointer = None;
            }
            Key::Space => self.paused = !self.paused,
            Key::Char('[') => {
                self.palette_index = self.palette_index.checked_sub(1).unwrap_or(2);
                self.set_palette();
            }
            Key::Char(']') => {
                self.palette_index = (self.palette_index + 1) % 3;
                self.set_palette();
            }
            _ => {}
        }
    }

    fn handle_pointer(&mut self, position: (u16, u16), phase: PointerPhase) {
        if matches!(phase, PointerPhase::WheelUp) {
            self.options.brush_radius = (self.options.brush_radius + 0.03).min(0.5);
        } else if matches!(phase, PointerPhase::WheelDown) {
            self.options.brush_radius =
                (self.options.brush_radius - 0.03).max(0.05);
        } else {
            let x = position.0 as f32 / (self.screen_size.0.max(2) - 1) as f32;
            let y = position.1 as f32 / (self.screen_size.1.max(2) - 1) as f32;
            self.pointer = Some(PointerEnergy {
                position: (x, y),
                strength: self.options.pointer_strength,
                radius: self.options.brush_radius,
            });
        }
    }

    /// Which of the three glyph sets is currently drawn, for a test.
    ///
    /// Exists because a test that reaches into `palette_index` from another
    /// module cannot, and the alternative -- a test that only says "the colours
    /// are still right" -- cannot say *which* palette switch lost them. Six
    /// presses of `]` is three glyph sets and three back round again, so a
    /// number here is what makes a failure legible.
    pub fn glyph_set_index(&self) -> usize {
        self.palette_index
    }

    fn decay_pointer(&mut self) {
        if let Some(pointer) = &mut self.pointer {
            pointer.strength *= 0.94;
            if pointer.strength < 0.01 {
                self.pointer = None;
            }
        }
    }

    /// Rebuilds the renderer for the current glyph set and configured colours.
    ///
    /// Three glyph sets, cycled by `[` and `]`, with index 0 being whatever
    /// `glyphs` says in the config file and the other two being the built-in
    /// alternatives. One colour ramp throughout: the two channels read the same
    /// field value, so the colour is not a third axis and switching glyph sets
    /// has no reason to change it.
    ///
    /// The configured colours are passed in rather than an empty vector, which is
    /// what makes `colors` a real knob: an empty vector is the "fall back to the
    /// phosphor ramp" signal, so before this, a configured `colors` list would
    /// have been accepted by the config and then discarded here.
    fn set_palette(&mut self) {
        const ALTERNATES: [&str; 2] = [" .:;+=xX#@", " .oO0#@"];
        let glyphs = match self.palette_index {
            0 => self.options.glyphs.as_str(),
            index => ALTERNATES[index - 1],
        };
        // `resolve_colors` rather than `self.options.colors`, so a `palette =`
        // name survives a `[`/`]` press. Reading the option directly here is
        // what would make the new knob work on the first frame and then quietly
        // revert the moment anyone pressed a key -- the exact bug
        // `cycling_the_glyph_set_keeps_the_configured_colours` was written for.
        self.renderer = AsciiRenderer::new(GlyphPalette::new(
            glyphs,
            resolve_colors(&self.options),
        ));
    }
}

/// The colour stops to paint with, given both knobs.
///
/// `palette` wins when it names something, because it is the spelling that says
/// what it means, and because the default `colors` is *always* populated -- so
/// the alternative is that the new option could never take effect for anyone
/// whose config file has the old key in it, which is everyone. The trade is
/// written out on [`AsciiFieldOptions::colors`].
///
/// An unknown name resolves to the default rather than to nothing, for the same
/// reason an empty `colors` list does: a typo in a hand-edited config should
/// give you the default effect, not a blank screen and no explanation. The
/// name is not case-sensitive, so `Orange` and `orange` are one name and not a
/// typo.
fn resolve_colors(options: &AsciiFieldOptions) -> Vec<Color> {
    if options.palette.trim().is_empty() {
        return options.colors.clone();
    }
    match super::renderer::palette_by_name(options.palette.trim()) {
        Some(stops) => stops.to_vec(),
        None => super::renderer::palette_by_name(DEFAULT_PALETTE_NAME)
            .expect("the default palette is in the table")
            .to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The user said it twice, and the first time it was not actually changed.
    ///
    /// "The white blobs are not fully white; they have a green tint after
    /// effects. They should be fully white or a single color." The top stop of
    /// the ramp was `rgb(230, 243, 227)`: the green channel sits 13 above the
    /// red and the blue 16 below it, so it is a 6.6% saturated green, and it is
    /// also only 243 -- not white, merely nearly so. Either alone is enough for
    /// the brightest parts of the field to read as green-cast.
    ///
    /// Measured on what the renderer actually draws rather than on a constant,
    /// because a constant can be right and the sampler can still be painting the
    /// wrong end of it. The glyph ramp's brightest entry is drawn on a real
    /// 120x40 frame and whatever colour comes out is the claim.
    #[test]
    fn the_brightest_cell_on_the_screen_is_neutral_white() {
        let mut field = AsciiField::new(AsciiFieldOptions::default(), (120, 40));
        let diff = field.get_diff();
        assert!(!diff.is_empty(), "the field drew nothing at all");

        let mut top: Option<(f32, (u8, u8, u8))> = None;
        for (_, _, cell) in &diff {
            let Color::Rgb { r, g, b } = cell.color else {
                panic!(
                    "the field drew {:?}, which is not a colour this effect can \
                     measure",
                    cell.color
                );
            };
            // Rec. 601 luma, so the pick does not depend on the ramp happening to
            // be one where the red channel is the highest.
            let luma =
                0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            if top.is_none_or(|(best, _)| luma > best) {
                top = Some((luma, (r, g, b)));
            }
        }
        let (r, g, b) = top.expect("no cell was drawn").1;

        let spread = r.max(g).max(b) - r.min(g).min(b);
        assert_eq!(
            spread, 0,
            "the whitest cell on the screen is rgb({r}, {g}, {b}): the channels \
             differ by {spread}, which is a {spread:.1}% green cast on the colour \
             the user reported as not white"
        );
        assert!(
            r >= 250,
            "the whitest cell is rgb({r}, {g}, {b}) at {r}/255, so the top of the \
             ramp never reaches white"
        );
    }

    /// Bold on a truecolor foreground is a brightening hint, and the ink field
    /// asked for it on exactly the cells the user was complaining about.
    ///
    /// The top sixth of the value range was `Attribute::Bold`. On a terminal
    /// that acts on the hint, that is a second brightness term applied on top of
    /// the one the ramp encodes, and applied *only* to the cells that are
    /// already at the top of the ramp -- so the brightest part of the field
    /// landed somewhere no amount of arithmetic could predict. Every other effect
    /// in this crate has taken the same option for the same reason, and this was
    /// the last one left.
    #[test]
    fn no_cell_is_asked_to_be_bright() {
        let mut field = AsciiField::new(AsciiFieldOptions::default(), (80, 24));
        for step in 0..8 {
            field.phase = step as f32 * 0.2;
            let diff = field.get_diff();
            assert!(!diff.is_empty(), "step {step} drew nothing");
            for (x, y, cell) in diff {
                assert_eq!(
                    cell.attr,
                    crossterm::style::Attribute::Reset,
                    "the cell at ({x}, {y}) is {cell:?}, asking the terminal to \
                     brighten a colour the ramp has already placed"
                );
            }
        }
    }

    /// The knob the user asked for: "fully white or a single color".
    ///
    /// A list of one is the single colour, and it has to survive a trip through
    /// a config file, because `--print-config` writes every key to disk and a
    /// generated config is pinned to whatever the defaults were the day it was
    /// generated. Every future knob arrives as "a key the user's file does not
    /// have", which is the normal case rather than the exotic one.
    #[test]
    fn the_monochrome_knob_round_trips_through_toml() {
        let config: AsciiFieldOptions =
            toml::from_str("colors = [\"rgb_(255,255,255)\"]\n")
                .expect("a single-colour list parses");
        assert_eq!(
            config.colors,
            vec![Color::Rgb {
                r: 255,
                g: 255,
                b: 255
            }],
            "the colour list was not read back"
        );
        assert_eq!(
            config.glyphs,
            AsciiFieldOptions::default().glyphs,
            "one key in the section silently reset the others"
        );

        // The other spellings a user will reach for, because the whole point of
        // the knob is that it is usable without reading this file.
        for (written, expected) in [
            (
                "colors = [\"#ffffff\"]\n",
                Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ),
            ("colors = [\"white\"]\n", Color::White),
            ("colors = [\"reset\"]\n", Color::Reset),
        ] {
            let parsed: AsciiFieldOptions = toml::from_str(written)
                .unwrap_or_else(|e| panic!("{written:?} did not parse: {e}"));
            assert_eq!(parsed.colors, vec![expected], "{written:?}");
        }

        let serialised = toml::to_string(&config).expect("the section serialises");
        assert!(
            serialised.contains("colors"),
            "the key is missing from the serialised form, so --print-config \
             would never write it: {serialised}"
        );

        // And it is *used*: a single-colour configuration paints one colour over
        // the whole field, at every value.
        let mut field = AsciiField::new(config, (60, 20));
        let diff = field.get_diff();
        let colors: HashSet<Color> =
            diff.iter().map(|(_, _, cell)| cell.color).collect();
        assert_eq!(
            colors,
            HashSet::from([Color::Rgb {
                r: 255,
                g: 255,
                b: 255
            }]),
            "a monochrome configuration painted {colors:?}, so the knob is not \
             reaching the renderer"
        );
        // ...while the glyphs still shade, so "monochrome" does not mean
        // "undifferentiated".
        let glyphs: HashSet<char> =
            diff.iter().map(|(_, _, cell)| cell.symbol).collect();
        assert!(
            glyphs.len() >= 3,
            "a monochrome field drew only {glyphs:?}, so the value is barely \
             reaching the ramp"
        );
    }

    /// An unusable value has to leave a drawable field, not a blank one.
    ///
    /// `colors = []` is what a hand-edited or generated config most easily ends
    /// up holding, and the alternative to falling back -- a palette with no
    /// colours in it -- is a screensaver that paints nothing and says nothing
    /// about why. The same has to hold for the glyph list, which had the same
    /// hazard before `GlyphPalette` grew its own fallbacks.
    #[test]
    fn an_unusable_colour_list_falls_back_rather_than_drawing_nothing() {
        for (label, options) in [
            (
                "empty colors",
                AsciiFieldOptions {
                    colors: Vec::new(),
                    ..Default::default()
                },
            ),
            (
                "empty glyphs",
                AsciiFieldOptions {
                    glyphs: String::new(),
                    ..Default::default()
                },
            ),
            (
                "both empty",
                AsciiFieldOptions {
                    glyphs: String::new(),
                    colors: Vec::new(),
                    ..Default::default()
                },
            ),
        ] {
            let mut field = AsciiField::new(options, (60, 20));
            let diff = field.get_diff();
            assert_eq!(
                diff.len(),
                60 * 20,
                "{label} drew {} cells of a 60x20 field, so part of the screen is \
                 not being drawn at all",
                diff.len()
            );
            let colors: HashSet<Color> =
                diff.iter().map(|(_, _, cell)| cell.color).collect();
            for color in &colors {
                assert!(
                    PHOSPHOR_RAMP.contains(color),
                    "{label} painted {color:?}, which is not a stop of the \
                     phosphor ramp, so the fallback did not happen"
                );
            }
            assert!(
                colors.contains(&Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255
                }),
                "{label} painted {colors:?}, and not one of them is white, so the \
                 ramp it fell back to is not the documented one"
            );
        }
    }

    /// `[` and `]` cycle the glyph set, and cycling must not lose the colours.
    ///
    /// The two channels are independent, and the tempting bug is a `set_palette`
    /// that rebuilds one from the options and the other from nothing -- which
    /// would make a configured `colors` list work on the first frame and revert
    /// to the phosphor ramp the moment anyone pressed a key.
    #[test]
    fn cycling_the_glyph_set_keeps_the_configured_colours() {
        let single = Color::Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        let mut field = AsciiField::new(
            AsciiFieldOptions {
                colors: vec![single],
                ..Default::default()
            },
            (40, 12),
        );
        for _ in 0..6 {
            field.handle_input(&InputEvent::Key {
                key: Key::Char(']'),
                phase: KeyPhase::Pressed,
            });
            let colors: HashSet<Color> = field
                .get_diff()
                .iter()
                .map(|(_, _, cell)| cell.color)
                .collect();
            assert_eq!(
                colors,
                HashSet::from([single]),
                "after cycling to glyph set {} the field painted {colors:?}, so \
                 the configured colours were dropped by the palette switch",
                field.palette_index
            );
        }
    }

    #[test]
    fn reset_preserves_interactive_state() {
        let mut field = AsciiField::new(AsciiFieldOptions::default(), (20, 10));
        field.handle_input(&InputEvent::Key {
            key: Key::Space,
            phase: KeyPhase::Pressed,
        });
        field.handle_input(&InputEvent::Key {
            key: Key::Char(']'),
            phase: KeyPhase::Pressed,
        });

        field.update_size(30, 12);
        field.reset();

        assert!(field.paused);
        assert_eq!(field.palette_index, 1);
    }
}

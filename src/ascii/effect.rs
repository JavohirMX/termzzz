use super::renderer::{AsciiRenderer, GlyphPalette};
use crate::buffer::{Buffer, Cell};
use crate::common::TerminalEffect;
use crate::runtime::{FrameContext, InputEvent, Key, KeyPhase, PointerPhase};
use crossterm::style::Color;
use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Builder, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[builder(public, setter(into))]
pub struct AsciiFieldOptions {
    #[builder(default = "42")]
    pub seed: u64,
    #[builder(default = "1.0")]
    pub time_scale: f32,
    #[builder(default = "0.7")]
    pub pointer_strength: f32,
    #[builder(default = "0.2")]
    pub brush_radius: f32,
    #[builder(default = "String::from(\" .:-=+*#%@\")")]
    pub glyphs: String,
}

impl Default for AsciiFieldOptions {
    fn default() -> Self {
        Self {
            seed: 42,
            time_scale: 1.0,
            pointer_strength: 0.7,
            brush_radius: 0.2,
            glyphs: String::from(" .:-=+*#%@"),
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
    buffer: Buffer,
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
        let previous = self.buffer.clone();
        self.render_frame();
        previous.diff(&self.buffer)
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
            InputEvent::Quit | InputEvent::Ignored => {}
        }
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        let width = self.screen_size.0 as usize;
        let height = self.screen_size.1 as usize;
        self.values = vec![0.0; width * height];
        self.buffer = Buffer::new(width, height);
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
        let renderer =
            AsciiRenderer::new(GlyphPalette::new(&options.glyphs, Vec::new()));
        let phase = Self::phase_for_seed(options.seed);

        Self {
            screen_size,
            values: vec![0.0; width * height],
            buffer: Buffer::new(width, height),
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

        self.renderer
            .render_field(&self.values, width, height, &mut self.buffer);
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

    fn decay_pointer(&mut self) {
        if let Some(pointer) = &mut self.pointer {
            pointer.strength *= 0.94;
            if pointer.strength < 0.01 {
                self.pointer = None;
            }
        }
    }

    fn set_palette(&mut self) {
        let palettes = [self.options.glyphs.as_str(), " .:;+=xX#@", " .oO0#@"];
        self.renderer = AsciiRenderer::new(GlyphPalette::new(
            palettes[self.palette_index],
            Vec::<Color>::new(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

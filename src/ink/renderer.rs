use crate::buffer::{Buffer, Cell};
use crossterm::style::{self, Color};

#[derive(Debug, Clone)]
pub struct GlyphPalette {
    glyphs: Vec<char>,
    colors: Vec<Color>,
}

impl Default for GlyphPalette {
    fn default() -> Self {
        Self {
            glyphs: " .:-=+*#%@".chars().collect(),
            colors: vec![
                Color::Rgb { r: 8, g: 32, b: 24 },
                Color::Rgb {
                    r: 16,
                    g: 88,
                    b: 54,
                },
                Color::Rgb {
                    r: 32,
                    g: 156,
                    b: 86,
                },
                Color::Rgb {
                    r: 83,
                    g: 212,
                    b: 144,
                },
                Color::Rgb {
                    r: 165,
                    g: 235,
                    b: 188,
                },
                Color::Rgb {
                    r: 230,
                    g: 243,
                    b: 227,
                },
            ],
        }
    }
}

impl GlyphPalette {
    pub fn new(glyphs: &str, colors: Vec<Color>) -> Self {
        let mut palette = Self {
            glyphs: glyphs
                .chars()
                .filter(|glyph| glyph.is_ascii() && !glyph.is_control())
                .collect(),
            colors,
        };
        if palette.glyphs.is_empty() {
            palette.glyphs = Self::default().glyphs;
        }
        if palette.colors.is_empty() {
            palette.colors = Self::default().colors;
        }
        palette
    }

    pub fn sample(&self, value: f32) -> Cell {
        let value = value.clamp(0.0, 1.0);
        let glyph_index = (value * (self.glyphs.len() - 1) as f32).round() as usize;
        let color_index = (value * (self.colors.len() - 1) as f32).round() as usize;
        let attribute = if value >= 0.72 {
            style::Attribute::Bold
        } else {
            style::Attribute::Reset
        };

        Cell::new(
            self.glyphs[glyph_index.min(self.glyphs.len() - 1)],
            self.colors[color_index.min(self.colors.len() - 1)],
            attribute,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct AsciiRenderer {
    palette: GlyphPalette,
}

impl AsciiRenderer {
    pub fn new(palette: GlyphPalette) -> Self {
        Self { palette }
    }

    pub fn render_field(
        &self,
        values: &[f32],
        width: usize,
        height: usize,
        buffer: &mut Buffer,
    ) {
        let width = width.min(buffer.width);
        let height = height.min(buffer.height);

        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let value = values.get(index).copied().unwrap_or_default();
                buffer.set(x, y, self.palette.sample(value));
            }
        }
    }
}

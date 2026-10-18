//! SVG fonts kept in the document (Type › SVG Font Editor): a font is a family name, its em size
//! in font units, a default advance and glyphs, each a character (or ligature string), a name,
//! an advance and its outline in font units (y up from the baseline), plus kerning pairs. The
//! SVG export writes them as `<font>` elements; the import reads them back.

use serde::{Deserialize, Serialize};

use redrob_vector_geom::PathData;

/// One glyph of an SVG font.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SvgGlyph {
    /// The character(s) the glyph draws (`unicode`).
    pub unicode: String,
    /// `glyph-name`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// `horiz-adv-x` in font units; `None`: the font's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<f64>,
    /// The outline in font units, y up from the baseline (as the `d` attribute is).
    pub path: PathData,
}

/// A kerning pair (`<hkern>`): the advance between `left` and `right` is cut by `k`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SvgKern {
    pub left: String,
    pub right: String,
    pub k: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SvgFont {
    /// The `<font>` id and the family name (`font-face font-family`).
    pub id: String,
    pub family: String,
    /// `units-per-em` (1000 by default).
    #[serde(default = "thousand")]
    pub units_per_em: f64,
    /// The default `horiz-adv-x`.
    #[serde(default = "thousand")]
    pub advance: f64,
    /// `ascent` / `descent` in font units (defaults: 80% and 20% of the em).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ascent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descent: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyphs: Vec<SvgGlyph>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kerning: Vec<SvgKern>,
}

fn thousand() -> f64 {
    1000.0
}

impl SvgFont {
    pub fn new(id: &str, family: &str) -> Self {
        Self {
            id: id.into(),
            family: family.into(),
            units_per_em: 1000.0,
            advance: 1000.0,
            ascent: None,
            descent: None,
            glyphs: vec![],
            kerning: vec![],
        }
    }

    pub fn glyph(&self, unicode: &str) -> Option<&SvgGlyph> {
        self.glyphs.iter().find(|g| g.unicode == unicode)
    }

    /// The advance of `unicode` (the glyph's, else the font's).
    pub fn advance_of(&self, unicode: &str) -> f64 {
        self.glyph(unicode).and_then(|g| g.advance).unwrap_or(self.advance)
    }

    /// The kerning between two glyphs (0 without a pair).
    pub fn kern(&self, left: &str, right: &str) -> f64 {
        self.kerning.iter().find(|k| k.left == left && k.right == right).map_or(0.0, |k| k.k)
    }

    /// Lay `text` out: each glyph with its pen position in font units (x from 0 on the baseline);
    /// characters without a glyph advance by the default and draw nothing.
    pub fn layout(&self, text: &str) -> Vec<(Option<&SvgGlyph>, f64)> {
        let mut out = vec![];
        let mut x = 0.0;
        let mut prev: Option<String> = None;
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // The longest ligature the font has at this point, else the single character.
            let mut taken = 1;
            let mut found: Option<&SvgGlyph> = None;
            for len in (1..=chars.len() - i).rev().take(4) {
                let s: String = chars[i..i + len].iter().collect();
                if let Some(g) = self.glyph(&s) {
                    found = Some(g);
                    taken = len;
                    break;
                }
            }
            let unicode: String = chars[i..i + taken].iter().collect();
            if let Some(p) = &prev {
                x -= self.kern(p, &unicode);
            }
            out.push((found, x));
            x += self.advance_of(&unicode);
            prev = Some(unicode);
            i += taken;
        }
        out
    }
}

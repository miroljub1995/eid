//! The print-out as an A4 PDF, a replica of the official "Čitač elektronske lične karte" print.
//!
//! Every position below was measured from a print made by the official reader (Windows, "Microsoft
//! Print to PDF"). That print is laid out in CSS pixels from the top-left corner and scaled by 0.75 to
//! points, and so is this one: with the same numbers in the same coordinate system, a rasteriser rounds
//! every edge the same way and the two prints match pixel for pixel.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use krilla::Document;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};

use eid_core::EidDocument;
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size as SkrifaSize};
use skrifa::outline::pen::NullPen;
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, Target};
use skrifa::{FontRef, MetadataProvider, OutlineGlyphCollection};

/// Liberation Sans (SIL OFL 1.1): metric-compatible with Arial, so the layout is identical when the
/// system has no Arial of its own. Covers Serbian Cyrillic and Latin.
pub const FONT_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Regular.ttf");

/// A4 as the official print declares it (595.32001 × 841.92004): rasterisers round these up, so the
/// width is the nearest f32 above 595.32 and both prints get the same pixel grid at every resolution.
const PAGE: (f32, f32) = (595.320_06, 841.920_04);
const PX: f32 = 0.75;

// Everything below is in CSS pixels.
const RULE_X: (f64, f64) = (79.36, 714.24);
/// Rules are thin filled rectangles with a 0.64 px outline, as in the original.
const RULE_STROKE: f32 = 0.64;
const THIN: f64 = 0.48;
const THICK: f64 = 0.8;

const TITLE: Style = Style {
    size: 17.6007,
    ppem: 110,
};
const BODY: Style = Style {
    size: 14.7206,
    ppem: 92,
};
const NOTE: Style = Style {
    size: 11.68,
    ppem: 73,
};
/// The original was laid out for a 600 dpi printer: 6.25 device pixels per CSS pixel.
const DEVICE_PER_PX: f32 = 6.25;

const TITLE_X: f64 = 91.68;
const LABEL_X: f64 = 89.76;
const VALUE_X: f64 = 260.96;
const NOTE_X: f64 = 79.36;
const LABEL_WIDTH: f32 = 160.0;
const VALUE_WIDTH: f32 = (RULE_X.1 - VALUE_X) as f32;
const NOTE_WIDTH: f32 = (RULE_X.1 - NOTE_X) as f32;

/// The photo frame's outline (centre line) and the photo inside it: left, top, right, bottom.
const FRAME: (f32, f32, f32, f32) = (78.72, 137.44, 238.56, 350.24);
const PHOTO: (f32, f32, f32, f32) = (79.36, 138.08, 237.92, 349.6);

/// Vertical rhythm, between a rule's top edge and baselines.
const RULE_TO_TEXT: f64 = 21.92;
const TEXT_TO_RULE: f64 = 10.88;
const ROW_STEP: f64 = 32.8;
const LINE_STEP: f64 = 16.48;
const DOUBLE_RULE_STEP: f64 = 3.84;
const FOOTER_RULE: f64 = 974.56;
const DATE_TO_FOOTER: f64 = 71.68;
const FOOTER_TO_NOTE: f64 = 20.64;
const NOTE_STEP: f64 = 13.12;
const NOTE_TO_RULE: f64 = 12.0;

/// A text size, and the same size in pixels per em on the original's 600 dpi device.
#[derive(Clone, Copy)]
struct Style {
    size: f32,
    ppem: u16,
}

/// `"Jovana-Petrović-lična-karta.pdf"`.
pub fn file_name(doc: &EidDocument) -> String {
    let name = doc.display_name().replace(' ', "-");
    if name.is_empty() {
        "lična-karta.pdf".into()
    } else {
        format!("{name}-lična-karta.pdf")
    }
}

/// Renders the print-out. `printed_on` is the date shown as "Datum štampe", e.g. `"05.01.2026."`.
pub fn render(doc: &EidDocument, printed_on: &str) -> Result<Vec<u8>, String> {
    let font_data = system_arial().map_or(Cow::Borrowed(FONT_REGULAR), Cow::Owned);
    let charmap = FontRef::new(&font_data)
        .map_err(|e| format!("the font: {e}"))?
        .charmap();
    let font = Font::new(font_data.to_vec().into(), 0).ok_or("the font is unreadable")?;
    let advances = Advances::new()?;
    let report = doc.report();

    let mut document = Document::new();
    document.set_metadata(
        Metadata::new()
            .title(format!(
                "Podaci iz lične karte osobe {} {}",
                doc.given_name, doc.surname
            ))
            .language("sr-Latn".into())
            .creator("eID čitač".into()),
    );
    let settings = PageSettings::from_wh(PAGE.0, PAGE.1).ok_or("bad page size")?;
    let mut page = document.start_page_with(settings);
    let mut surface = page.surface();
    let mut pen = Pen {
        surface: &mut surface,
        font,
        charmap,
        advances,
    };
    pen.surface.push_transform(&Transform::from_scale(PX, PX));

    pen.rule(79.04, THIN);
    pen.text(TITLE_X, 104.96, TITLE, &report.title.to_uppercase());
    pen.rule(118.24, THIN);
    pen.photo(&doc.photo);

    let mut rule = 368.64;
    pen.rule(rule, THICK);
    for section in &report.sections {
        let title = rule + RULE_TO_TEXT;
        pen.text(LABEL_X, title, BODY, section.title);
        rule = title + TEXT_TO_RULE;
        pen.rule(rule, THICK);
        let mut baseline = rule + RULE_TO_TEXT;
        let mut last = baseline;
        for row in &section.rows {
            let label = pen.paragraph(
                LABEL_X,
                baseline,
                LABEL_WIDTH,
                BODY,
                LINE_STEP,
                &format!("{}:", row.label),
            );
            let value = pen.paragraph(VALUE_X, baseline, VALUE_WIDTH, BODY, LINE_STEP, &row.value);
            last = label.max(value);
            baseline = last + ROW_STEP;
        }
        rule = last + TEXT_TO_RULE;
        pen.rule(rule, THICK);
    }
    rule += DOUBLE_RULE_STEP;
    pen.rule(rule, THICK);
    let date = rule + RULE_TO_TEXT;
    pen.text(LABEL_X, date, BODY, &format!("Datum štampe: {printed_on}"));

    let footer = FOOTER_RULE.max(date + DATE_TO_FOOTER);
    pen.rule(footer, THIN);
    // shortcut: one page only; a residence permit with very long values could run off the bottom.
    let last = pen.paragraph(
        NOTE_X,
        footer + FOOTER_TO_NOTE,
        NOTE_WIDTH,
        NOTE,
        NOTE_STEP,
        report.note,
    );
    pen.rule(last + NOTE_TO_RULE, THIN);
    pen.surface.pop();

    surface.finish();
    page.finish();
    document
        .finish()
        .map_err(|e| format!("could not write the PDF: {e:?}"))
}

/// The official print is set in Arial. Where the system has it, use it (read at run time, never
/// shipped): the glyphs then match exactly. Otherwise Liberation Sans keeps every position the same.
fn system_arial() -> Option<Vec<u8>> {
    for path in [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/usr/share/fonts/truetype/msttcorefonts/Arial.ttf",
        "/usr/share/fonts/msttcore/arial.ttf",
        "/usr/share/fonts/TTF/arial.ttf",
    ] {
        if let Ok(data) = std::fs::read(path)
            && FontRef::new(&data).is_ok_and(|f| f.charmap().map('Ж').is_some())
        {
            return Some(data);
        }
    }
    None
}

/// Glyph advances as Windows GDI laid out the official print: each glyph's width hinted (TrueType,
/// monochrome) at the 600 dpi printer resolution and rounded to whole device pixels. Liberation Sans
/// hints to the same widths as Arial for every character the print uses, so this works without Arial.
struct Advances {
    glyphs: OutlineGlyphCollection<'static>,
    charmap: Charmap<'static>,
    instances: HashMap<u16, HintingInstance>,
    cache: HashMap<(u16, char), f32>,
}

impl Advances {
    fn new() -> Result<Self, String> {
        let font = FontRef::new(FONT_REGULAR).map_err(|e| format!("the built-in font: {e}"))?;
        let (glyphs, charmap) = (font.outline_glyphs(), font.charmap());
        Ok(Advances {
            glyphs,
            charmap,
            instances: HashMap::new(),
            cache: HashMap::new(),
        })
    }

    /// The advance of `c` in CSS pixels.
    fn get(&mut self, style: Style, c: char) -> f32 {
        if let Some(advance) = self.cache.get(&(style.ppem, c)) {
            return *advance;
        }
        let gid = self.charmap.map(c).unwrap_or_default();
        let instance = match self.instances.entry(style.ppem) {
            Entry::Occupied(e) => Some(e.into_mut()),
            Entry::Vacant(e) => {
                let options = HintingOptions {
                    engine: Engine::Interpreter,
                    target: Target::Mono,
                };
                let size = SkrifaSize::new(f32::from(style.ppem));
                HintingInstance::new(&self.glyphs, size, LocationRef::default(), options)
                    .ok()
                    .map(|i| e.insert(i))
            }
        };
        let hinted = instance.and_then(|instance| {
            let glyph = self.glyphs.get(gid)?;
            glyph
                .draw(DrawSettings::hinted(instance, false), &mut NullPen)
                .ok()?
                .advance_width
        });
        let pixels = hinted.unwrap_or_else(|| {
            let metrics = FontRef::new(FONT_REGULAR)
                .map(|f| f.glyph_metrics(SkrifaSize::new(f32::from(style.ppem)), LocationRef::default()));
            metrics.ok().and_then(|m| m.advance_width(gid)).unwrap_or(0.0)
        });
        let advance = pixels.round() / DEVICE_PER_PX;
        self.cache.insert((style.ppem, c), advance);
        advance
    }
}

struct Pen<'a, 'b, 'f> {
    surface: &'a mut Surface<'b>,
    /// The font the glyphs are drawn with (Arial when the system has it) and its character map.
    font: Font,
    charmap: Charmap<'f>,
    advances: Advances,
}

impl Pen<'_, '_, '_> {
    /// Places each glyph at GDI's position for it (see [`Advances`]); no kerning, as in the original.
    /// Layout runs in f64 and each coordinate becomes the f32 nearest the exact value, as in the original.
    fn text(&mut self, x: f64, baseline: f64, style: Style, text: &str) {
        let glyphs: Vec<KrillaGlyph> = text
            .char_indices()
            .map(|(i, c)| {
                let id = self.charmap.map(c).unwrap_or_default();
                let advance = self.advances.get(style, c) / style.size;
                KrillaGlyph::new(
                    GlyphId::new(id.to_u32()),
                    advance,
                    0.0,
                    0.0,
                    0.0,
                    i..i + c.len_utf8(),
                    None,
                )
            })
            .collect();
        self.surface.set_stroke(None);
        self.surface.set_fill(Some(Fill::default()));
        let start = Point::from_xy(x as f32, baseline as f32);
        self.surface
            .draw_glyphs(start, &glyphs, self.font.clone(), text, style.size, false);
    }

    fn width(&mut self, text: &str, style: Style) -> f32 {
        text.chars().map(|c| self.advances.get(style, c)).sum()
    }

    /// Word-wrapped text from `baseline`; returns the baseline of its last line.
    fn paragraph(&mut self, x: f64, baseline: f64, width: f32, style: Style, step: f64, text: &str) -> f64 {
        let lines = wrap(text, width, |s| self.width(s, style));
        let mut y = baseline;
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                y += step;
            }
            self.text(x, y, style, line);
        }
        y
    }

    /// A full-width rule whose filled rectangle starts at `top`.
    fn rule(&mut self, top: f64, height: f64) {
        let rect = Rect::from_ltrb(
            RULE_X.0 as f32,
            top as f32,
            RULE_X.1 as f32,
            (top + height) as f32,
        );
        let stroke = Stroke {
            width: RULE_STROKE,
            ..Default::default()
        };
        self.draw_rect(rect, Some(Fill::default()), stroke);
    }

    fn photo(&mut self, jpeg: &[u8]) {
        let (l, t, r, b) = PHOTO;
        if let Ok(image) = Image::from_jpeg(jpeg.to_vec().into(), true)
            && let Some(size) = Size::from_wh(r - l, b - t)
        {
            self.surface.push_transform(&Transform::from_translate(l, t));
            self.surface.draw_image(image, size);
            self.surface.pop();
        }
        let (l, t, r, b) = FRAME;
        self.draw_rect(
            Rect::from_ltrb(l, t, r, b),
            None,
            Stroke {
                width: RULE_STROKE,
                ..Default::default()
            },
        );
    }

    fn draw_rect(&mut self, rect: Option<Rect>, fill: Option<Fill>, stroke: Stroke) {
        let mut path = PathBuilder::new();
        let Some(rect) = rect else { return };
        path.push_rect(rect);
        let Some(path) = path.finish() else { return };
        self.surface.set_fill(fill);
        self.surface.set_stroke(Some(stroke));
        self.surface.draw_path(&path);
        self.surface.set_stroke(None);
    }
}

/// Greedy word wrap; a word wider than the column gets a line of its own.
fn wrap(text: &str, width: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) > width && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_like_the_official_label_column() {
        let mut advances = Advances::new().unwrap();
        let lines = wrap("Mesto rođenja, opština i država:", LABEL_WIDTH, |s| {
            s.chars().map(|c| advances.get(BODY, c)).sum()
        });
        assert_eq!(lines, ["Mesto rođenja, opština i", "država:"]);
        assert!(wrap("", 10.0, |s| s.len() as f32).is_empty());
        assert_eq!(
            wrap("toolongword x", 3.0, |s| s.len() as f32),
            ["toolongword", "x"]
        );
    }

    #[test]
    fn advances_match_the_official_print() {
        // "Poda" from the official print's body text, in CSS pixels (pt / 0.75): where GDI put each
        // glyph, which differs from Arial's design advances (9.82, 8.19, 8.19, 8.19).
        let mut advances = Advances::new().unwrap();
        for (c, expected) in [('P', 9.716), ('o', 8.187), ('d', 8.187), ('a', 8.096)] {
            let px = advances.get(BODY, c);
            assert!((px - expected).abs() < 0.1, "{c}: {px} vs {expected}");
        }
    }

    #[test]
    fn renders_a_pdf() {
        let doc = EidDocument {
            given_name: "ЈОВАНА".into(),
            surname: "ПЕТРОВИЋ".into(),
            ..Default::default()
        };
        let pdf = render(&doc, "05.01.2026.").unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(file_name(&doc), "Jovana-Petrović-lična-karta.pdf");
    }
}

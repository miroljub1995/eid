//! The print-out as an A4 PDF, laid out like the official "Čitač elektronske lične karte".

use krilla::Document;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::page::PageSettings;
use krilla::paint::Stroke;
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};

use eid_core::EidDocument;

/// Liberation Sans (SIL OFL 1.1): metric-compatible with Arial, which the official print-out uses,
/// and it covers Serbian Cyrillic and Latin.
pub const FONT_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Regular.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Bold.ttf");

const PAGE: (f32, f32) = (595.28, 841.89);
const LEFT: f32 = 58.8;
const RIGHT: f32 = 535.0;
const TEXT_LEFT: f32 = 67.3;
const VALUE_LEFT: f32 = TEXT_LEFT + 128.0;
const LABEL_WIDTH: f32 = 120.0;
const VALUE_WIDTH: f32 = RIGHT - VALUE_LEFT;
const LINE_HEIGHT: f32 = 12.0;
const ROW_GAP: f32 = 24.67;
const PHOTO: (f32, f32) = (119.9, 159.0);

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
    let font = Font::new(FONT_REGULAR.into(), 0).ok_or("the built-in font is unreadable")?;
    let metrics = Metrics::new(FONT_REGULAR)?;
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
        metrics,
        size: 13.5,
        y: 0.0,
    };

    pen.y = 59.04;
    pen.rule(0.83);
    pen.y = 68.5;
    pen.text(TEXT_LEFT + 1.0, &report.title.to_uppercase());
    pen.y = 88.0;
    pen.rule(0.83);

    let photo_top = 102.8;
    if let Ok(image) = Image::from_jpeg(doc.photo.clone().into(), true)
        && let Some(size) = Size::from_wh(PHOTO.0, PHOTO.1)
    {
        pen.surface
            .push_transform(&Transform::from_translate(LEFT, photo_top));
        pen.surface.draw_image(image, size);
        pen.surface.pop();
    }
    pen.frame(LEFT, photo_top, PHOTO.0, PHOTO.1, 0.48);

    pen.y = 276.0;
    pen.rule(1.08);
    pen.size = 11.1;
    for (i, section) in report.sections.iter().enumerate() {
        if i > 0 {
            pen.y -= ROW_GAP - 16.0;
            pen.rule(0.83);
        }
        pen.y += 8.0;
        pen.text(TEXT_LEFT, section.title);
        pen.y += 16.0;
        pen.rule(0.83);
        pen.y += 9.0;
        for row in &section.rows {
            let top = pen.y;
            let label_bottom = pen.paragraph(TEXT_LEFT, LABEL_WIDTH, &format!("{}:", row.label));
            pen.y = top;
            let value_bottom = pen.paragraph(VALUE_LEFT, VALUE_WIDTH, &row.value);
            pen.y = label_bottom.max(value_bottom) + ROW_GAP;
        }
    }
    pen.y -= ROW_GAP - 16.0;
    pen.rule(0.83);
    pen.y += 3.0;
    pen.rule(0.83);
    pen.y += 9.0;
    pen.text(TEXT_LEFT, &format!("Datum štampe: {printed_on}"));

    pen.y = (pen.y + 19.0).max(730.6);
    pen.rule(0.83);
    pen.size = 9.0;
    pen.y += 6.0;
    let note_bottom = pen.paragraph_spaced(LEFT, RIGHT - LEFT, report.note, 9.7);
    pen.y = note_bottom + 15.7;
    pen.rule(0.83);

    surface.finish();
    page.finish();
    document
        .finish()
        .map_err(|e| format!("could not write the PDF: {e:?}"))
}

/// Advance widths, for wrapping text to a column.
struct Metrics {
    advances: Vec<(char, u16)>,
    default: u16,
    units_per_em: f32,
    ascender: f32,
}

impl Metrics {
    fn new(data: &[u8]) -> Result<Self, String> {
        let face = ttf_parser::Face::parse(data, 0).map_err(|e| format!("the built-in font: {e}"))?;
        let advance = |c: char| face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g));
        // Precompute what the print-out can contain: ASCII, Latin-1, Latin Extended-A and Cyrillic.
        let advances = (' '..='\u{17F}')
            .chain('\u{400}'..='\u{4FF}')
            .filter_map(|c| advance(c).map(|a| (c, a)))
            .collect();
        Ok(Metrics {
            advances,
            default: advance('n').unwrap_or(0),
            units_per_em: f32::from(face.units_per_em()),
            ascender: f32::from(face.ascender()),
        })
    }

    fn width(&self, text: &str, size: f32) -> f32 {
        let units: u32 = text
            .chars()
            .map(|c| {
                self.advances
                    .iter()
                    .find(|(k, _)| *k == c)
                    .map_or(self.default, |(_, a)| *a)
            })
            .map(u32::from)
            .sum();
        units as f32 * size / self.units_per_em
    }
}

/// Draws top-down: `y` is the top of the next line of text, as on the official print-out's grid.
struct Pen<'a, 'b> {
    surface: &'a mut Surface<'b>,
    font: Font,
    metrics: Metrics,
    size: f32,
    y: f32,
}

impl Pen<'_, '_> {
    fn text(&mut self, x: f32, text: &str) {
        let baseline = self.y + self.metrics.ascender / self.metrics.units_per_em * self.size;
        let at = Point::from_xy(x, baseline);
        self.surface
            .draw_text(at, self.font.clone(), self.size, text, false, TextDirection::Auto);
    }

    /// Word-wrapped text; returns the top of its last line.
    fn paragraph(&mut self, x: f32, width: f32, text: &str) -> f32 {
        self.paragraph_spaced(x, width, text, LINE_HEIGHT)
    }

    fn paragraph_spaced(&mut self, x: f32, width: f32, text: &str, line_height: f32) -> f32 {
        let lines = wrap(text, width, |s| self.metrics.width(s, self.size));
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                self.y += line_height;
            }
            self.text(x, line);
        }
        self.y
    }

    /// A full-width horizontal rule at `y`.
    fn rule(&mut self, width: f32) {
        let mut path = PathBuilder::new();
        path.move_to(LEFT, self.y);
        path.line_to(RIGHT, self.y);
        self.stroke(path, width);
    }

    fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32) {
        let mut path = PathBuilder::new();
        path.move_to(x, y);
        path.line_to(x + w, y);
        path.line_to(x + w, y + h);
        path.line_to(x, y + h);
        path.close();
        self.stroke(path, width);
    }

    fn stroke(&mut self, path: PathBuilder, width: f32) {
        let Some(path) = path.finish() else { return };
        self.surface.set_fill(None);
        self.surface.set_stroke(Some(Stroke {
            width,
            ..Default::default()
        }));
        self.surface.draw_path(&path);
        self.surface.set_stroke(None);
    }
}

/// Greedy word wrap; a word wider than the column gets a line of its own.
fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
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
        let metrics = Metrics::new(FONT_REGULAR).unwrap();
        let lines = wrap("Mesto rođenja, opština i država:", LABEL_WIDTH, |s| {
            metrics.width(s, 11.1)
        });
        assert_eq!(lines, ["Mesto rođenja, opština i", "država:"]);
        assert!(wrap("", 10.0, |s| s.len() as f32).is_empty());
        assert_eq!(
            wrap("toolongword x", 3.0, |s| s.len() as f32),
            ["toolongword", "x"]
        );
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

//! Colours, type and widget styling. The views take colours from [`Tokens`], never literals.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Theme};

#[derive(Clone, Copy)]
pub(crate) struct Tokens {
    pub bg: Color32,
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub danger: Color32,
}

impl Tokens {
    pub(crate) fn of(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode { DARK } else { LIGHT }
    }
}

const LIGHT: Tokens = Tokens {
    bg: Color32::from_rgb(0xF3, 0xF4, 0xF6),
    surface: Color32::WHITE,
    border: Color32::from_rgb(0xE1, 0xE4, 0xE8),
    text: Color32::from_rgb(0x1B, 0x1F, 0x24),
    muted: Color32::from_rgb(0x5F, 0x67, 0x73),
    accent: Color32::from_rgb(0x0B, 0x4F, 0x9C),
    on_accent: Color32::WHITE,
    ok: Color32::from_rgb(0x13, 0x7A, 0x4B),
    warn: Color32::from_rgb(0xB2, 0x6B, 0x00),
    danger: Color32::from_rgb(0xB4, 0x23, 0x18),
};

const DARK: Tokens = Tokens {
    bg: Color32::from_rgb(0x14, 0x16, 0x1A),
    surface: Color32::from_rgb(0x1E, 0x21, 0x26),
    border: Color32::from_rgb(0x33, 0x38, 0x40),
    text: Color32::from_rgb(0xE8, 0xEA, 0xED),
    muted: Color32::from_rgb(0x9A, 0xA1, 0xAB),
    // Dark enough for white button text (4.8:1), light enough to read on `bg` as an outline.
    accent: Color32::from_rgb(0x35, 0x72, 0xC8),
    on_accent: Color32::WHITE,
    ok: Color32::from_rgb(0x4C, 0xC2, 0x8A),
    warn: Color32::from_rgb(0xE5, 0xA5, 0x3C),
    danger: Color32::from_rgb(0xF0, 0x6A, 0x5E),
};

pub(crate) const BOLD: &str = "bold";
/// Liberation Sans Bold (SIL OFL 1.1); the regular weight comes from the PDF crate, which needs it too.
const FONT_BOLD: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Bold.ttf");

pub(crate) fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::empty();
    for (name, data) in [("regular", eid_pdf::FONT_REGULAR), (BOLD, FONT_BOLD)] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(data).into());
    }
    fonts
        .families
        .insert(FontFamily::Proportional, vec!["regular".into()]);
    fonts
        .families
        .insert(FontFamily::Monospace, vec!["regular".into()]);
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), vec![BOLD.into()]);
    ctx.set_fonts(fonts);

    for (theme, t) in [(Theme::Light, LIGHT), (Theme::Dark, DARK)] {
        ctx.style_mut_of(theme, |style| {
            style.text_styles = [
                (TextStyle::Small, FontId::proportional(12.0)),
                (TextStyle::Body, FontId::proportional(14.0)),
                (TextStyle::Button, FontId::proportional(14.0)),
                (TextStyle::Monospace, FontId::monospace(13.0)),
                (
                    TextStyle::Heading,
                    FontId::new(22.0, FontFamily::Name(BOLD.into())),
                ),
            ]
            .into();
            style.spacing.button_padding = egui::vec2(14.0, 7.0);
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);

            let v = &mut style.visuals;
            v.panel_fill = t.bg;
            v.window_fill = t.surface;
            v.extreme_bg_color = t.surface;
            v.faint_bg_color = t.bg;
            v.override_text_color = Some(t.text);
            v.hyperlink_color = t.accent;
            v.selection.bg_fill = t.accent.gamma_multiply(0.35);
            v.selection.stroke = Stroke::new(1.0, t.accent);
            for w in [
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
            ] {
                w.corner_radius = CornerRadius::same(6);
            }
            v.widgets.inactive.weak_bg_fill = t.surface;
            v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.accent);
        });
    }
}

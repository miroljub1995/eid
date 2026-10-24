//! The viewer's interface. Platform specifics come in through [`Host`] and the status channel, so the
//! same UI runs on the desktop (PC/SC) and in the browser (WebUSB).

mod theme;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use chrono::{Local, NaiveDate};
use egui::{
    Align, Button, Color32, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText,
    Sense, Stroke, TextureHandle, Ui, vec2,
};

use eid_core::{Command, DocumentKind, EidDocument, Error, Status};
use theme::{BOLD, Tokens};

/// What the UI needs from the platform it runs on.
pub trait Host {
    /// Lets the user save `bytes` as `name` (a save dialog on the desktop, a download in the browser).
    /// Returns where it went, or `None` if the user cancelled.
    fn save_file(&self, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String>;

    /// Whether readers must be granted to the app one by one (WebUSB) before they show up.
    fn can_pair_reader(&self) -> bool {
        false
    }

    /// Asks the user to grant a reader. Only called when [`Host::can_pair_reader`] is true, from a click.
    fn pair_reader(&self) {}
}

pub struct EidApp {
    host: Box<dyn Host>,
    statuses: Receiver<Status>,
    commands: Sender<Command>,
    status: Status,
    photo: Option<TextureHandle>,
    notice: Option<Notice>,
}

struct Notice {
    text: String,
    error: bool,
    until: f64,
}

/// Wide enough for "Mesto rođenja, opština i država", so both sections' values line up.
const LABEL_WIDTH: f32 = 230.0;
/// Below this width the layout stacks: header actions under the title, photo above the name,
/// labels above their values.
const NARROW: f32 = 640.0;
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const REREAD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);

impl EidApp {
    /// `statuses` comes from [`eid_core::run`], which `commands` drives.
    pub fn new(
        ctx: &egui::Context,
        host: Box<dyn Host>,
        statuses: Receiver<Status>,
        commands: Sender<Command>,
    ) -> Self {
        theme::install(ctx);
        EidApp {
            host,
            statuses,
            commands,
            status: Status::NoReader,
            photo: None,
            notice: None,
        }
    }

    fn take_statuses(&mut self, ctx: &egui::Context) {
        while let Ok(status) = self.statuses.try_recv() {
            // The photo lives exactly as long as the card's data is on screen.
            self.photo = match &status {
                Status::Ready { document, .. } => photo_texture(ctx, &document.photo),
                _ => None,
            };
            self.status = status;
        }
    }

    fn document(&self) -> Option<&Arc<EidDocument>> {
        match &self.status {
            Status::Ready { document, .. } => Some(document),
            _ => None,
        }
    }

    fn save_pdf(&mut self, now: f64) {
        let Some(doc) = self.document() else { return };
        let today = Local::now().format("%d.%m.%Y.").to_string();
        let result =
            eid_pdf::render(doc, &today).and_then(|pdf| self.host.save_file(&eid_pdf::file_name(doc), pdf));
        let (text, error) = match result {
            Ok(Some(place)) => (format!("PDF je sačuvan: {place}"), false),
            Ok(None) => return,
            Err(error) => (format!("PDF nije sačuvan: {error}"), true),
        };
        self.notice = Some(Notice {
            text,
            error,
            until: now + 6.0,
        });
    }

    fn reread(&self) {
        // The watcher only stops when the app closes, so a send cannot fail while the UI is up.
        self.commands.send(Command::Reread).ok();
    }

    /// Wide: title and actions on one row. Narrow: the title alone, the actions on a row below it.
    fn header(&mut self, ui: &mut Ui, t: Tokens, narrow: bool) {
        if narrow {
            ui.horizontal(|ui| title(ui, t, false));
            ui.add_space(2.0);
        }
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            if !narrow {
                title(ui, t, true);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.actions(ui, t, !narrow)
            });
        });
    }

    /// Laid out right to left, so the primary action comes first.
    fn actions(&mut self, ui: &mut Ui, t: Tokens, shortcuts: bool) {
        let now = ui.input(|i| i.time);
        let has_document = self.document().is_some();
        let has_card = matches!(
            self.status,
            Status::Ready { .. } | Status::Failed { reader: Some(_), .. }
        );
        let save_keys = ui.ctx().format_shortcut(&SAVE);
        let mut save = if has_document {
            primary_button(t, "Sačuvaj PDF")
        } else {
            Button::new("Sačuvaj PDF")
        };
        if shortcuts {
            let color = if has_document {
                t.on_accent.gamma_multiply(0.75)
            } else {
                t.muted
            };
            save = save.shortcut_text(RichText::new(&save_keys).color(color));
        }
        if ui
            .add_enabled(has_document, save)
            .on_hover_text(format!(
                "Sačuvaj podatke kao PDF, u obliku zvaničnog izveštaja ({save_keys})"
            ))
            .on_disabled_hover_text("Dostupno kada se kartica učita")
            .clicked()
        {
            self.save_pdf(now);
        }
        let reread_keys = ui.ctx().format_shortcut(&REREAD);
        let mut reread = Button::new("Učitaj ponovo");
        if shortcuts {
            reread = reread.shortcut_text(RichText::new(&reread_keys).color(t.muted));
        }
        if ui
            .add_enabled(has_card, reread)
            .on_hover_text(format!("Ponovo pročitaj karticu iz čitača ({reread_keys})"))
            .on_disabled_hover_text("Dostupno kada je kartica u čitaču")
            .clicked()
        {
            self.reread();
        }
        // With no reader at all, the empty state offers pairing instead.
        let pair = self.host.can_pair_reader() && !matches!(self.status, Status::NoReader);
        if pair
            && ui
                .button("Poveži čitač")
                .on_hover_text("Dozvoli pregledaču pristup još jednom USB čitaču")
                .clicked()
        {
            self.host.pair_reader();
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, t: Tokens) {
        let now = ui.input(|i| i.time);
        let (color, text) = match &self.status {
            Status::NoReader => (t.muted, "Čitač nije pronađen".to_owned()),
            Status::NoCard { reader } => (t.warn, format!("{reader} · nema kartice")),
            Status::Reading { reader } => (t.accent, format!("{reader} · čitanje…")),
            Status::Ready { reader, .. } => (t.ok, format!("{reader} · kartica je učitana")),
            Status::Failed { reader, .. } => (
                t.danger,
                format!("{} · greška", reader.as_deref().unwrap_or("Čitač")),
            ),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (dot, response) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, color);
            response.on_hover_text("Stanje čitača");
            ui.add(egui::Label::new(RichText::new(text).small().color(t.muted)).truncate());
            if let Some(notice) = self.notice.as_ref().filter(|n| n.until > now) {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let color = if notice.error { t.danger } else { t.ok };
                    ui.add(egui::Label::new(RichText::new(&notice.text).small().color(color)).truncate());
                });
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(notice.until - now));
            }
        });
    }

    fn body(&mut self, ui: &mut Ui, t: Tokens, narrow: bool) {
        match &self.status {
            Status::Ready { document, .. } => {
                let document = Arc::clone(document);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    centered_column(ui, 860.0, |ui| {
                        document_view(ui, t, &document, self.photo.as_ref(), narrow)
                    });
                });
            }
            Status::Reading { .. } => {
                let hint = "Ne vadite karticu iz čitača.";
                empty_state(ui, t, Art::Spinner, "Čitanje kartice…", hint, None, None);
            }
            Status::NoCard { .. } => {
                let hint = "Stavite karticu u čitač, čipom napred. Podaci će se prikazati automatski.";
                empty_state(
                    ui,
                    t,
                    Art::Card(t.accent),
                    "Ubacite ličnu kartu",
                    hint,
                    None,
                    None,
                );
            }
            Status::NoReader => {
                let pair = self.host.can_pair_reader();
                let hint = if pair {
                    "Priključite USB čitač kartica i dozvolite pregledaču pristup njemu."
                } else {
                    "Priključite čitač kartica na računar."
                };
                let action = pair.then_some("Poveži čitač");
                if empty_state(
                    ui,
                    t,
                    Art::Card(t.muted),
                    "Čitač nije pronađen",
                    hint,
                    None,
                    action,
                ) {
                    self.host.pair_reader();
                }
            }
            Status::Failed { reader, error } => {
                let (title, hint) = describe(error, reader.is_some());
                // The technical cause is for support, so it sits apart from the advice.
                let detail = (!matches!(error, Error::UnknownCard(_))).then(|| format!("Detalji: {error}"));
                let action = reader.is_some().then_some("Pokušaj ponovo");
                if empty_state(ui, t, Art::Card(t.danger), title, hint, detail.as_deref(), action) {
                    self.reread();
                }
            }
        }
    }
}

impl eframe::App for EidApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_statuses(&ctx);
        let now = ui.input(|i| i.time);
        if ui.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.save_pdf(now);
        }
        if ui.input_mut(|i| i.consume_shortcut(&REREAD)) {
            self.reread();
        }

        let t = Tokens::of(ui);
        let narrow = ui.available_width() < NARROW;
        let bar = |m: Margin| {
            Frame::new()
                .fill(t.surface)
                .inner_margin(m)
                .stroke(Stroke::new(1.0, t.border))
        };
        egui::Panel::top("header")
            .frame(bar(Margin::symmetric(gutter(narrow), 10)))
            .show(ui, |ui| self.header(ui, t, narrow));
        egui::Panel::bottom("status")
            .frame(bar(Margin::symmetric(gutter(narrow), 6)))
            .show(ui, |ui| self.status_bar(ui, t));
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(t.bg))
            .show(ui, |ui| self.body(ui, t, narrow));
    }
}

fn document_view(ui: &mut Ui, t: Tokens, doc: &EidDocument, photo: Option<&TextureHandle>, narrow: bool) {
    ui.add_space(if narrow { 16.0 } else { 24.0 });
    surface(t, narrow).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let layout = if narrow {
            Layout::top_down(Align::Min)
        } else {
            Layout::left_to_right(Align::Min)
        };
        ui.with_layout(layout, |ui| {
            portrait(ui, t, photo);
            ui.add_space(if narrow { 4.0 } else { 12.0 });
            ui.vertical(|ui| holder(ui, t, doc));
        });
    });

    let report = doc.report();
    for section in &report.sections {
        ui.add_space(16.0);
        surface(t, narrow).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(section.title.to_uppercase())
                    .small()
                    .family(bold())
                    .color(t.muted),
            );
            ui.add_space(6.0);
            if narrow {
                stacked_rows(ui, t, &section.rows);
            } else {
                grid_rows(ui, t, section.title, &section.rows);
            }
        });
    }
    ui.add_space(12.0);
    ui.add(egui::Label::new(RichText::new(report.note).small().color(t.muted)).wrap());
    ui.add_space(24.0);
}

fn portrait(ui: &mut Ui, t: Tokens, photo: Option<&TextureHandle>) {
    let size = vec2(132.0, 176.0);
    match photo {
        Some(texture) => {
            ui.add(egui::Image::new(texture).fit_to_exact_size(size).corner_radius(8))
                .on_hover_text("Fotografija sa čipa");
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().rect_filled(rect, 8, t.bg);
        }
    }
}

/// Document kind, name and the at-a-glance chips beside the portrait.
fn holder(ui: &mut Ui, t: Tokens, doc: &EidDocument) {
    ui.add_space(4.0);
    let kind = match doc.kind {
        DocumentKind::IdCard => "Lična karta",
        DocumentKind::ForeignerIdCard => "Lična karta za strance",
        DocumentKind::ResidencePermit => "Dozvola za privremeni boravak i rad",
    };
    ui.label(
        RichText::new(kind.to_uppercase())
            .small()
            .family(bold())
            .color(t.muted),
    );
    let name = format!("{} {}", doc.given_name, doc.surname);
    let latin = doc.display_name();
    let show_latin = !latin.is_empty() && latin != name;
    ui.add(egui::Label::new(RichText::new(name).heading()).wrap());
    if show_latin {
        ui.label(RichText::new(latin).color(t.muted));
    }
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        if !doc.personal_number.is_empty() {
            chip(ui, t, &format!("JMBG {}", doc.personal_number), t.muted);
        }
        if !doc.doc_reg_no.is_empty() {
            chip(ui, t, &format!("Br. dokumenta {}", doc.doc_reg_no), t.muted);
        }
        let expiry = eid_core::format_date(&doc.expiry_date);
        match NaiveDate::parse_from_str(&doc.expiry_date, "%d%m%Y") {
            Ok(date) if date < Local::now().date_naive() => {
                chip(ui, t, &format!("Istekla {expiry}"), t.danger);
            }
            Ok(_) => chip(ui, t, &format!("Važi do {expiry}"), t.ok),
            Err(_) => {}
        }
    });
}

fn grid_rows(ui: &mut Ui, t: Tokens, id: &str, rows: &[eid_core::Row]) {
    egui::Grid::new(id)
        .num_columns(2)
        .min_col_width(LABEL_WIDTH)
        .spacing(vec2(24.0, 10.0))
        .show(ui, |ui| {
            for row in rows {
                ui.label(RichText::new(row.label).color(t.muted));
                value(ui, &row.value);
                ui.end_row();
            }
        });
}

/// Label above value, for windows too narrow for two columns.
fn stacked_rows(ui: &mut Ui, t: Tokens, rows: &[eid_core::Row]) {
    for row in rows {
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(row.label).small().color(t.muted));
        value(ui, &row.value);
        ui.add_space(6.0);
    }
}

fn value(ui: &mut Ui, value: &str) {
    let value = if value.is_empty() { "—" } else { value };
    ui.add(egui::Label::new(RichText::new(value).size(15.0)).wrap());
}

fn surface(t: Tokens, narrow: bool) -> Frame {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(if narrow { 16 } else { 20 }))
}

/// A pill that never wraps, so `horizontal_wrapped` moves it to the next line whole.
fn chip(ui: &mut Ui, t: Tokens, text: &str, color: Color32) {
    let text_color = if color == t.muted { t.text } else { color };
    let galley = egui::WidgetText::from(RichText::new(text).small().color(text_color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Small,
    );
    let pad = vec2(10.0, 3.0);
    let (rect, response) = ui.allocate_exact_size(galley.size() + 2.0 * pad, Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    ui.painter().rect_filled(rect, 10, color.gamma_multiply(0.12));
    ui.painter().galley(rect.min + pad, galley, text_color);
}

fn centered_column(ui: &mut Ui, max_width: f32, add: impl FnOnce(&mut Ui)) {
    let width = (ui.available_width() - 32.0).clamp(0.0, max_width);
    let side = (ui.available_width() - width) / 2.0;
    ui.horizontal(|ui| {
        ui.add_space(side);
        ui.vertical(|ui| {
            ui.set_width(width);
            add(ui);
        });
    });
}

enum Art {
    Card(Color32),
    Spinner,
}

/// A centred message; returns whether its action button was clicked.
fn empty_state(
    ui: &mut Ui,
    t: Tokens,
    art: Art,
    title: &str,
    hint: &str,
    detail: Option<&str>,
    action: Option<&str>,
) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 120.0).max(24.0));
        match art {
            Art::Card(color) => card_icon(ui, 72.0, color),
            Art::Spinner => {
                ui.add(egui::Spinner::new().size(48.0).color(t.accent));
            }
        }
        ui.add_space(16.0);
        centered_column(ui, 440.0, |ui| {
            ui.vertical_centered(|ui| {
                ui.add(egui::Label::new(RichText::new(title).heading()).wrap());
                ui.add(egui::Label::new(RichText::new(hint).color(t.muted)).wrap());
                if let Some(detail) = detail {
                    ui.add(egui::Label::new(RichText::new(detail).small().color(t.muted)).wrap());
                }
                if let Some(action) = action {
                    ui.add_space(8.0);
                    clicked = ui.add(primary_button(t, action)).clicked();
                }
            });
        });
    });
    clicked
}

fn primary_button(t: Tokens, text: &str) -> Button<'static> {
    Button::new(RichText::new(text).color(t.on_accent))
        .fill(t.accent)
        .stroke(Stroke::NONE)
}

/// The app's name beside its mark; `subtitle` only when there is room for it.
fn title(ui: &mut Ui, t: Tokens, subtitle: bool) {
    card_icon(ui, 26.0, t.accent);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(
            RichText::new("Čitač elektronske lične karte")
                .family(bold())
                .size(16.0),
        );
        if subtitle {
            ui.label(
                RichText::new("Prikaz i štampa podataka sa čipa")
                    .small()
                    .color(t.muted),
            );
        }
    });
}

/// Side margin of the header and status bar, matching the content column's.
fn gutter(narrow: bool) -> i8 {
    if narrow { 16 } else { 20 }
}

/// An ID card with a chip, drawn so the app ships no image assets.
fn card_icon(ui: &mut Ui, width: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 0.63), Sense::hover());
    let p = ui.painter();
    let r = (width * 0.08) as u8;
    p.rect_stroke(
        rect.shrink(1.0),
        r,
        Stroke::new((width / 24.0).max(1.5), color),
        egui::StrokeKind::Inside,
    );
    let unit = width / 10.0;
    let chip = egui::Rect::from_min_size(
        rect.min + vec2(unit * 1.2, unit * 2.0),
        vec2(unit * 2.2, unit * 1.7),
    );
    p.rect_filled(chip, (unit * 0.3) as u8, color.gamma_multiply(0.7));
    for (i, w) in [5.0, 3.6].into_iter().enumerate() {
        let y = rect.min.y + unit * (2.4 + 1.0 * i as f32);
        let line = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + unit * 4.4, y),
            vec2(unit * w * 0.9, unit * 0.45),
        );
        p.rect_filled(line, (unit * 0.2) as u8, color.gamma_multiply(0.45));
    }
}

fn describe(error: &Error, reading: bool) -> (&'static str, &'static str) {
    match error {
        Error::UnknownCard(_) => (
            "Kartica nije prepoznata",
            "Ovo nije srpska lična karta, ili ovaj tip kartice još nije podržan.",
        ),
        _ if reading => (
            "Čitanje kartice nije uspelo",
            "Izvadite karticu i ubacite je ponovo.",
        ),
        _ => ("Čitač nije dostupan", "Proverite da li je čitač priključen."),
    }
}

fn bold() -> egui::FontFamily {
    egui::FontFamily::Name(BOLD.into())
}

/// The card's portrait is a few hundred pixels; anything far bigger is not a real photo and could
/// exhaust memory or exceed the GPU's texture limit, so it is not shown.
const MAX_PHOTO_SIDE: u32 = 2048;

fn photo_texture(ctx: &egui::Context, jpeg: &[u8]) -> Option<TextureHandle> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PHOTO_SIDE);
    limits.max_image_height = Some(MAX_PHOTO_SIDE);
    reader.limits(limits);
    let image = reader.decode().ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture("photo", pixels, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let image = image::GrayImage::new(width, height);
        image
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
            .unwrap();
        out
    }

    #[test]
    fn refuses_photos_no_card_could_hold() {
        let ctx = egui::Context::default();
        assert!(photo_texture(&ctx, &jpeg(240, 320)).is_some());
        // Would trip egui's texture size assertion (a panic) and allocate far too much.
        assert!(photo_texture(&ctx, &jpeg(1, 20_000)).is_none());
        assert!(photo_texture(&ctx, b"not a jpeg").is_none());
    }
}

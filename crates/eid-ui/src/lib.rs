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

    fn header(&mut self, ui: &mut Ui, t: Tokens) {
        let now = ui.input(|i| i.time);
        let has_card = matches!(
            self.status,
            Status::Ready { .. } | Status::Failed { reader: Some(_), .. }
        );
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            card_icon(ui, 26.0, t.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    RichText::new("Čitač elektronske lične karte")
                        .family(bold())
                        .size(16.0),
                );
                ui.label(
                    RichText::new("Prikaz i štampa podataka sa čipa")
                        .small()
                        .color(t.muted),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let save = Button::new(RichText::new("Sačuvaj PDF").color(t.on_accent))
                    .fill(t.accent)
                    .stroke(Stroke::NONE)
                    .shortcut_text(
                        RichText::new(ui.ctx().format_shortcut(&SAVE)).color(t.on_accent.gamma_multiply(0.7)),
                    );
                if ui
                    .add_enabled(self.document().is_some(), save)
                    .on_hover_text("Sačuvaj podatke kao PDF, u obliku zvaničnog izveštaja")
                    .clicked()
                {
                    self.save_pdf(now);
                }
                let reread = Button::new("Učitaj ponovo").shortcut_text(ui.ctx().format_shortcut(&REREAD));
                if ui.add_enabled(has_card, reread).clicked() {
                    self.reread();
                }
                if self.host.can_pair_reader() && ui.button("Poveži čitač").clicked() {
                    self.host.pair_reader();
                }
            });
        });
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
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, color);
            ui.label(RichText::new(text).small().color(t.muted));
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

    fn body(&mut self, ui: &mut Ui, t: Tokens) {
        match &self.status {
            Status::Ready { document, .. } => {
                let document = Arc::clone(document);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    centered_column(ui, 860.0, |ui| {
                        document_view(ui, t, &document, self.photo.as_ref())
                    });
                });
            }
            Status::Reading { .. } => {
                let hint = "Ne vadite karticu iz čitača.";
                empty_state(ui, t, Art::Spinner, "Čitanje kartice…", hint, None);
            }
            Status::NoCard { .. } => {
                let hint = "Stavite karticu u čitač, čipom napred. Podaci će se prikazati automatski.";
                empty_state(ui, t, Art::Card(t.accent), "Ubacite ličnu kartu", hint, None);
            }
            Status::NoReader => {
                let pair = self.host.can_pair_reader();
                let hint = if pair {
                    "Priključite USB čitač kartica i dozvolite pregledaču pristup njemu."
                } else {
                    "Priključite čitač kartica na računar."
                };
                let action = pair.then_some("Poveži čitač");
                if empty_state(ui, t, Art::Card(t.muted), "Čitač nije pronađen", hint, action) {
                    self.host.pair_reader();
                }
            }
            Status::Failed { reader, error } => {
                let (title, hint) = describe(error, reader.is_some());
                let action = reader.is_some().then_some("Pokušaj ponovo");
                if empty_state(ui, t, Art::Card(t.danger), title, &hint, action) {
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
        let bar = |m: Margin| {
            Frame::new()
                .fill(t.surface)
                .inner_margin(m)
                .stroke(Stroke::new(1.0, t.border))
        };
        egui::Panel::top("header")
            .frame(bar(Margin::symmetric(20, 10)))
            .show(ui, |ui| self.header(ui, t));
        egui::Panel::bottom("status")
            .frame(bar(Margin::symmetric(20, 6)))
            .show(ui, |ui| self.status_bar(ui, t));
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(t.bg))
            .show(ui, |ui| self.body(ui, t));
    }
}

fn document_view(ui: &mut Ui, t: Tokens, doc: &EidDocument, photo: Option<&TextureHandle>) {
    ui.add_space(24.0);
    surface(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let size = vec2(132.0, 176.0);
            match photo {
                Some(texture) => {
                    ui.add(egui::Image::new(texture).fit_to_exact_size(size).corner_radius(8));
                }
                None => {
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    ui.painter().rect_filled(rect, 8, t.bg);
                }
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                let kind = match doc.kind {
                    DocumentKind::IdCard => "Lična karta",
                    DocumentKind::ForeignerIdCard => "Lična karta za strance",
                    DocumentKind::ResidencePermit => "Dozvola za privremeni boravak i rad",
                };
                ui.label(RichText::new(kind.to_uppercase()).small().color(t.muted));
                let name = format!("{} {}", doc.given_name, doc.surname);
                ui.add(egui::Label::new(RichText::new(name).heading()).wrap());
                let latin = doc.display_name();
                if !latin.is_empty() && latin != format!("{} {}", doc.given_name, doc.surname) {
                    ui.label(RichText::new(latin).color(t.muted));
                }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if !doc.personal_number.is_empty() {
                        chip(ui, t, &format!("JMBG {}", doc.personal_number), t.muted);
                    }
                    if !doc.doc_reg_no.is_empty() {
                        chip(ui, t, &format!("Br. dokumenta {}", doc.doc_reg_no), t.muted);
                    }
                    match NaiveDate::parse_from_str(&doc.expiry_date, "%d%m%Y") {
                        Ok(expiry) if expiry < Local::now().date_naive() => chip(
                            ui,
                            t,
                            &format!("Istekla {}", eid_core::format_date(&doc.expiry_date)),
                            t.danger,
                        ),
                        Ok(_) => chip(
                            ui,
                            t,
                            &format!("Važi do {}", eid_core::format_date(&doc.expiry_date)),
                            t.ok,
                        ),
                        Err(_) => {}
                    }
                });
            });
        });
    });

    let report = doc.report();
    for section in &report.sections {
        ui.add_space(16.0);
        surface(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(section.title.to_uppercase())
                    .small()
                    .family(bold())
                    .color(t.muted),
            );
            ui.add_space(6.0);
            egui::Grid::new(section.title)
                .num_columns(2)
                .min_col_width(LABEL_WIDTH)
                .spacing(vec2(24.0, 10.0))
                .show(ui, |ui| {
                    for row in &section.rows {
                        ui.label(RichText::new(row.label).color(t.muted));
                        let value = if row.value.is_empty() { "—" } else { &row.value };
                        ui.add(egui::Label::new(RichText::new(value).size(15.0)).wrap());
                        ui.end_row();
                    }
                });
        });
    }
    ui.add_space(12.0);
    ui.label(RichText::new(report.note).small().color(t.muted));
    ui.add_space(24.0);
}

fn surface(t: Tokens) -> Frame {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(20))
}

fn chip(ui: &mut Ui, t: Tokens, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .small()
                    .color(if color == t.muted { t.text } else { color }),
            )
        });
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
fn empty_state(ui: &mut Ui, t: Tokens, art: Art, title: &str, hint: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 120.0).max(24.0));
        match art {
            Art::Card(color) => card_icon(ui, 72.0, color),
            Art::Spinner => {
                ui.add(egui::Spinner::new().size(48.0).color(t.accent));
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new(title).heading());
        ui.add(egui::Label::new(RichText::new(hint).color(t.muted)).wrap());
        if let Some(action) = action {
            ui.add_space(8.0);
            let button = Button::new(RichText::new(action).color(t.on_accent))
                .fill(t.accent)
                .stroke(Stroke::NONE);
            clicked = ui.add(button).clicked();
        }
    });
    clicked
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

fn describe(error: &Error, reading: bool) -> (&'static str, String) {
    match error {
        Error::UnknownCard(_) => (
            "Kartica nije prepoznata",
            "Ovo nije srpska lična karta, ili je ovaj tip kartice još nepodržan.".into(),
        ),
        _ if reading => (
            "Čitanje kartice nije uspelo",
            format!("Izvadite karticu i ubacite je ponovo. ({error})"),
        ),
        _ => (
            "Čitač nije dostupan",
            format!("Proverite da li je čitač priključen. ({error})"),
        ),
    }
}

fn bold() -> egui::FontFamily {
    egui::FontFamily::Name(BOLD.into())
}

fn photo_texture(ctx: &egui::Context, jpeg: &[u8]) -> Option<TextureHandle> {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .ok()?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture("photo", pixels, egui::TextureOptions::LINEAR))
}

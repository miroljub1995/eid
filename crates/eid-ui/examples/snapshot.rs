//! Renders every screen offscreen to PNGs, light and dark, with a made-up holder.
//!
//! `cargo run -p eid-ui --example snapshot -- <out-dir>`

// A developer tool: failing loudly is the right behaviour.
#![allow(clippy::expect_used)]

use std::sync::{Arc, mpsc};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use eid_core::{DocumentKind, EidDocument, Error, Event, Reader, Status};
use eid_ui::{EidApp, Host};

struct NoHost;

impl Host for NoHost {
    fn save_file(&self, _: &str, _: Vec<u8>) -> Result<Option<String>, String> {
        Ok(None)
    }
}

fn sample() -> EidDocument {
    // A plain grey gradient stands in for the portrait.
    let portrait = image::RgbImage::from_fn(300, 400, |_, y| image::Rgb([(140 + y / 8) as u8; 3]));
    let mut photo = Vec::new();
    portrait
        .write_to(&mut std::io::Cursor::new(&mut photo), image::ImageFormat::Jpeg)
        .expect("jpeg");
    EidDocument {
        kind: DocumentKind::IdCard,
        surname: "ПЕТРОВИЋ".into(),
        given_name: "ЈОВАНА".into(),
        parent_given_name: "MARKO".into(),
        date_of_birth: "15031990".into(),
        place_of_birth: "KRAGUJEVAC".into(),
        community_of_birth: "KRAGUJEVAC".into(),
        state_of_birth: "REPUBLIKA SRBIJA".into(),
        place: "NIŠ".into(),
        community: "NIŠ-MEDIJANA".into(),
        street: "KNEZA MILOŠA".into(),
        house_number: "12".into(),
        floor: "1".into(),
        apartment_number: "2".into(),
        address_date: "02122022".into(),
        address_label: "prebivalište".into(),
        personal_number: "1503990000000".into(),
        sex: "Ž".into(),
        issuing_authority: "ПУ У НИШУ".into(),
        doc_reg_no: "001234567".into(),
        issuing_date: "09102021".into(),
        expiry_date: "09102031".into(),
        photo,
        ..Default::default()
    }
}

fn main() {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/snapshots".into()),
    );
    std::fs::create_dir_all(&out).expect("output dir");
    let reader = "Generic EMV Smartcard Reader".to_owned();
    let foreigner = EidDocument {
        kind: DocumentKind::ForeignerIdCard,
        surname: "VAN DER BERG-SCHMIDT".into(),
        given_name: "ALEXANDRA MARIA".into(),
        expiry_date: "01012020".into(),
        ..sample()
    };
    let states = [
        ("no-reader", Status::NoReader),
        (
            "no-card",
            Status::NoCard {
                reader: reader.clone(),
            },
        ),
        (
            "reading",
            Status::Reading {
                reader: reader.clone(),
            },
        ),
        (
            "ready",
            Status::Ready {
                reader: reader.clone(),
                document: Arc::new(sample()),
            },
        ),
        (
            "foreigner-expired",
            Status::Ready {
                reader: reader.clone(),
                document: Arc::new(foreigner),
            },
        ),
        (
            "failed",
            Status::Failed {
                reader: Some(reader.clone()),
                error: Error::Transport("Card was removed".into()),
            },
        ),
        (
            "unknown-card",
            Status::Failed {
                reader: Some(reader.clone()),
                error: Error::UnknownCard("3B00".into()),
            },
        ),
    ];
    let one = vec![Reader {
        name: reader.clone(),
        has_card: true,
    }];
    let two = vec![
        Reader {
            name: reader.clone(),
            has_card: true,
        },
        Reader {
            name: "Gemplus USB SmartCard Reader".into(),
            has_card: false,
        },
    ];
    let ready = Status::Ready {
        reader: reader.clone(),
        document: Arc::new(sample()),
    };
    let mut scenes: Vec<(&str, Status, Vec<Reader>, bool)> = states
        .into_iter()
        .map(|(name, status)| {
            let readers = if matches!(status, Status::NoReader) {
                vec![]
            } else {
                one.clone()
            };
            (name, status, readers, false)
        })
        .collect();
    scenes.push(("two-readers", ready.clone(), two.clone(), false));
    scenes.push(("two-readers-open", ready, two, true));
    for (name, status, readers, open_picker) in scenes {
        for (theme, suffix, size) in [
            (egui::Theme::Light, "", [960.0, 820.0]),
            (egui::Theme::Dark, "-dark", [960.0, 820.0]),
            (egui::Theme::Light, "-narrow", [420.0, 760.0]),
            (egui::Theme::Dark, "-narrow-dark", [420.0, 760.0]),
        ] {
            let (tx, rx) = mpsc::channel();
            let (commands, _keep) = mpsc::channel();
            tx.send(Event::Readers(readers.clone())).expect("send");
            tx.send(Event::Status(status.clone())).expect("send");
            let mut harness = Harness::builder()
                .with_size(size)
                .with_theme(theme)
                .wgpu()
                .build_eframe(|cc| EidApp::new(&cc.egui_ctx, Box::new(NoHost), rx, commands));
            harness.run_steps(4);
            if open_picker {
                harness.get_by_label("Čitač iz koga se čita kartica").click();
                harness.run_steps(2);
            }
            let image = harness.render().expect("render");
            image.save(out.join(format!("{name}{suffix}.png"))).expect("save");
        }
    }
    println!("wrote {}", out.display());
}

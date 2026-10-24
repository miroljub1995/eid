//! Renders every screen offscreen to PNGs, light and dark, with a made-up holder.
//!
//! `cargo run -p eid-ui --example snapshot -- <out-dir>`

use std::sync::{Arc, mpsc};

use egui_kittest::Harness;
use eid_core::{DocumentKind, EidDocument, Error, Status};
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
            "failed",
            Status::Failed {
                reader: Some(reader.clone()),
                error: Error::Transport("Card was removed".into()),
            },
        ),
        (
            "unknown-card",
            Status::Failed {
                reader: Some(reader),
                error: Error::UnknownCard("3B00".into()),
            },
        ),
    ];
    for (name, status) in states {
        for (theme, suffix, size) in [
            (egui::Theme::Light, "", [960.0, 820.0]),
            (egui::Theme::Dark, "-dark", [960.0, 820.0]),
            (egui::Theme::Light, "-narrow", [420.0, 760.0]),
        ] {
            let (tx, rx) = mpsc::channel();
            let (commands, _keep) = mpsc::channel();
            tx.send(status.clone()).expect("send");
            let mut harness = Harness::builder()
                .with_size(size)
                .with_theme(theme)
                .wgpu()
                .build_eframe(|cc| EidApp::new(&cc.egui_ctx, Box::new(NoHost), rx, commands));
            harness.run_steps(4);
            let image = harness.render().expect("render");
            image.save(out.join(format!("{name}{suffix}.png"))).expect("save");
        }
    }
    println!("wrote {}", out.display());
}

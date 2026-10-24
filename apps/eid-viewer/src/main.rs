//! The desktop app: PC/SC readers, native save dialog.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc;

use eid_pcsc::Pcsc;
use eid_ui::{EidApp, Host};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Čitač elektronske lične karte")
            .with_inner_size([960.0, 820.0])
            .with_min_inner_size([560.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "eid-viewer",
        options,
        Box::new(|cc| {
            let (status_tx, status_rx) = mpsc::channel();
            let (command_tx, command_rx) = mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            // PC/SC calls block, so the watcher gets its own thread; it ends when the UI drops `command_tx`.
            std::thread::Builder::new()
                .name("card-watcher".into())
                .spawn(move || {
                    pollster::block_on(eid_core::run(Pcsc::new(), command_rx, |status| {
                        if status_tx.send(status).is_ok() {
                            ctx.request_repaint();
                        }
                    }));
                })?;
            Ok(Box::new(EidApp::new(
                &cc.egui_ctx,
                Box::new(Desktop),
                status_rx,
                command_tx,
            )))
        }),
    )
}

struct Desktop;

impl Host for Desktop {
    fn save_file(&self, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return Ok(None);
        };
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(Some(path.display().to_string()))
    }
}

//! The iOS/iPadOS app: card readers through CryptoTokenKit, PDFs saved to the app's Documents folder.
//! Build and package it with `apps/eid-ios/bundle.sh` (needs Xcode).

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("eid-ios runs on iOS: build it with apps/eid-ios/bundle.sh");
}

#[cfg(target_os = "ios")]
fn main() -> eframe::Result {
    ios::run()
}

#[cfg(target_os = "ios")]
mod ios {
    use std::path::PathBuf;
    use std::sync::mpsc;

    use eid_ctk::CryptoTokenKit;
    use eid_ui::{EidApp, Host};

    pub(crate) fn run() -> eframe::Result {
        eframe::run_native(
            "eid-ios",
            eframe::NativeOptions::default(),
            Box::new(|cc| {
                let (event_tx, events) = mpsc::channel();
                let (commands, command_rx) = mpsc::channel();
                let ctx = cc.egui_ctx.clone();
                // CryptoTokenKit calls block until it answers, so the watcher gets its own thread; it
                // ends when the UI drops `commands`.
                std::thread::Builder::new()
                    .name("card-watcher".into())
                    .spawn(move || {
                        pollster::block_on(eid_core::run(CryptoTokenKit::new(), command_rx, |event| {
                            if event_tx.send(event).is_ok() {
                                ctx.request_repaint();
                            }
                        }));
                    })?;
                Ok(Box::new(Viewer(EidApp::new(
                    &cc.egui_ctx,
                    Box::new(Phone),
                    events,
                    commands,
                ))))
            }),
        )
    }

    struct Viewer(EidApp);

    impl eframe::App for Viewer {
        fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
            self.0.show(ui);
        }
    }

    struct Phone;

    impl Host for Phone {
        /// Saves into the app's Documents folder, which the Files app shows under the app's name
        /// (UIFileSharingEnabled and LSSupportsOpeningDocumentsInPlace in Info.plist).
        // shortcut: no share sheet; presenting UIActivityViewController needs UIKit glue.
        fn save_file(&self, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
            let home = std::env::var_os("HOME").ok_or("the app has no home folder")?;
            let documents = PathBuf::from(home).join("Documents");
            std::fs::create_dir_all(&documents).map_err(|e| e.to_string())?;
            std::fs::write(documents.join(name), bytes).map_err(|e| e.to_string())?;
            Ok(Some(format!("Fajlovi › eID › {name}")))
        }
    }
}

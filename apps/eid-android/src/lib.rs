//! The Android app: USB card readers through the USB host API, PDFs saved to the app's external
//! files folder. Package it with `cargo apk build -p eid-android` (needs the Android SDK and NDK).

#![cfg(target_os = "android")]

use std::path::PathBuf;
use std::sync::mpsc;

use eid_android_usb::Android;
use eid_ui::{EidApp, Host};
use winit::platform::android::activity::AndroidApp;

// The Android activity calls this by name.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let logs = android_logger::Config::default()
        .with_tag("eid")
        .with_max_level(log::LevelFilter::Info);
    android_logger::init_once(logs);
    std::panic::set_hook(Box::new(|info| log::error!("{info}")));
    let documents = app.external_data_path();
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    let result = eframe::run_native(
        "eid",
        options,
        Box::new(move |cc| {
            let (event_tx, events) = mpsc::channel();
            let (commands, command_rx) = mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            // USB transfers block, so the watcher gets its own thread; it ends when the UI drops
            // `commands`.
            std::thread::Builder::new()
                .name("card-watcher".into())
                .spawn(move || {
                    let platform = match Android::new() {
                        Ok(platform) => platform,
                        Err(error) => {
                            log::error!("USB host unavailable: {error}");
                            let status = eid_core::Status::Failed { reader: None, error };
                            event_tx.send(eid_core::Event::Status(status)).ok();
                            return;
                        }
                    };
                    pollster::block_on(eid_core::run(platform, command_rx, |event| {
                        if event_tx.send(event).is_ok() {
                            ctx.request_repaint();
                        }
                    }));
                })?;
            let host = Box::new(Phone { documents });
            Ok(Box::new(Viewer(EidApp::new(
                &cc.egui_ctx,
                host,
                events,
                commands,
            ))))
        }),
    );
    if let Err(error) = result {
        log::error!("the app failed: {error}");
    }
}

struct Viewer(EidApp);

impl eframe::App for Viewer {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(ui);
    }
}

struct Phone {
    documents: Option<PathBuf>,
}

impl Host for Phone {
    /// Saves into the app's external files folder (Android/data/rs.eid.viewer/files).
    // shortcut: no share sheet or MediaStore Downloads entry; both need more Java glue.
    fn save_file(&self, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
        let folder = self.documents.as_ref().ok_or("the app has no files folder")?;
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let path = folder.join(name);
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(Some(path.display().to_string()))
    }
}

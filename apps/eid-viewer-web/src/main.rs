//! The browser app: USB readers through WebUSB, the PDF as a download. Build with `trunk build`.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("eid-viewer-web runs in the browser: build it with `trunk build` in apps/eid-viewer-web.");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::sync::mpsc;

    use eid_ui::{EidApp, Host};
    use eid_webusb::WebUsb;
    use wasm_bindgen::JsCast;

    pub(crate) fn start() {
        wasm_bindgen_futures::spawn_local(async {
            let Some(canvas) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("app"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            else {
                web_sys::console::error_1(&"no <canvas id=\"app\">".into());
                return;
            };
            let started = eframe::WebRunner::new()
                .start(
                    canvas,
                    eframe::WebOptions::default(),
                    Box::new(|cc| {
                        let (status_tx, status_rx) = mpsc::channel();
                        let (command_tx, command_rx) = mpsc::channel();
                        let ctx = cc.egui_ctx.clone();
                        if eid_webusb::is_supported() {
                            wasm_bindgen_futures::spawn_local(eid_core::run(
                                WebUsb::new(),
                                command_rx,
                                move |s| {
                                    if status_tx.send(s).is_ok() {
                                        ctx.request_repaint();
                                    }
                                },
                            ));
                        } else {
                            let error = eid_core::Error::Transport(
                                "Ovaj pregledač ne podržava WebUSB. Otvorite stranicu u Chrome-u ili Edge-u."
                                    .into(),
                            );
                            status_tx
                                .send(eid_core::Status::Failed { reader: None, error })
                                .ok();
                        }
                        Ok(Box::new(EidApp::new(
                            &cc.egui_ctx,
                            Box::new(Browser),
                            status_rx,
                            command_tx,
                        )))
                    }),
                )
                .await;
            if let Err(error) = started {
                web_sys::console::error_2(&"the app failed to start:".into(), &error);
            }
        });
    }

    struct Browser;

    impl Host for Browser {
        fn save_file(&self, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
            let fail = |e: wasm_bindgen::JsValue| format!("{e:?}");
            let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
            let options = web_sys::BlobPropertyBag::new();
            options.set_type("application/pdf");
            let blob =
                web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(fail)?;
            let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(fail)?;
            let document = web_sys::window()
                .and_then(|w| w.document())
                .ok_or("no document")?;
            let link: web_sys::HtmlAnchorElement = document
                .create_element("a")
                .map_err(fail)?
                .dyn_into()
                .map_err(|_| "no <a>")?;
            link.set_href(&url);
            link.set_download(name);
            link.click();
            web_sys::Url::revoke_object_url(&url).map_err(fail)?;
            Ok(Some(name.to_owned()))
        }

        fn can_pair_reader(&self) -> bool {
            eid_webusb::is_supported()
        }

        fn pair_reader(&self) {
            // Started inside the click's frame, while the browser still counts the user's activation.
            wasm_bindgen_futures::spawn_local(async {
                if let Err(error) = eid_webusb::request_reader().await {
                    web_sys::console::error_1(&error.to_string().into());
                }
            });
        }
    }
}

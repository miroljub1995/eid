//! `eid` — wait for a card, read it and print it. Uses the same reader watcher as the GUI.
//!
//! Usage: `eid [--pdf out.pdf] [--photo out.jpg]`

use std::process::ExitCode;

use std::sync::mpsc;

use eid_core::Status;
use eid_pcsc::Pcsc;

fn main() -> ExitCode {
    match read() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eid: {error}");
            ExitCode::FAILURE
        }
    }
}

fn read() -> Result<(), String> {
    let (mut pdf_path, mut photo_path) = (None, None);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--pdf", Some(path)) => pdf_path = Some(path),
            ("--photo", Some(path)) => photo_path = Some(path),
            _ => return Err("usage: eid [--pdf out.pdf] [--photo out.jpg]".into()),
        }
    }

    let (status_tx, statuses) = mpsc::channel();
    let (commands, command_rx) = mpsc::channel();
    // The watcher stops once `commands` is dropped at the end of this function.
    std::thread::spawn(move || {
        pollster::block_on(eid_core::run(Pcsc::new(), command_rx, |s| {
            status_tx.send(s).ok();
        }))
    });
    let doc = loop {
        match statuses.recv().map_err(|_| "the reader watcher stopped")? {
            Status::NoReader => eprintln!("Waiting for a card reader…"),
            Status::NoCard { reader } => eprintln!("{reader}: waiting for a card…"),
            Status::Reading { reader } => eprintln!("{reader}: reading…"),
            Status::Ready { document, .. } => break document,
            Status::Failed { error, .. } => return Err(error.to_string()),
        }
    };
    drop(commands);

    let report = doc.report();
    println!("{}", report.title.to_uppercase());
    for section in &report.sections {
        println!("\n{}", section.title);
        for row in &section.rows {
            println!("  {:<34}{}", format!("{}:", row.label), row.value);
        }
    }
    if let Some(path) = pdf_path {
        let today = chrono::Local::now().format("%d.%m.%Y.").to_string();
        let pdf = eid_pdf::render(&doc, &today)?;
        std::fs::write(&path, pdf).map_err(|e| format!("{path}: {e}"))?;
    }
    if let Some(path) = photo_path {
        std::fs::write(&path, &doc.photo).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(())
}

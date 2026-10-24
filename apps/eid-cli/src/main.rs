//! `eid` — wait for a card, read it and print it. Uses the same reader watcher as the GUI.
//!
//! Usage: `eid [--reader NAME] [--pdf out.pdf] [--photo out.jpg]`. Without `--reader` it reads the
//! first reader that holds a card.

use std::process::ExitCode;

use std::sync::mpsc;

use eid_core::{Command, Event, Status};
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
    let (mut pdf_path, mut photo_path, mut reader) = (None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--pdf", Some(path)) => pdf_path = Some(path),
            ("--photo", Some(path)) => photo_path = Some(path),
            ("--reader", Some(name)) => reader = Some(name),
            _ => return Err("usage: eid [--reader NAME] [--pdf out.pdf] [--photo out.jpg]".into()),
        }
    }

    let (status_tx, statuses) = mpsc::channel();
    let (commands, command_rx) = mpsc::channel();
    // Sent before the watcher starts, so it never reads another reader first.
    commands
        .send(Command::Select(reader.clone()))
        .map_err(|_| "the reader watcher stopped")?;
    // The watcher stops once `commands` is dropped at the end of this function.
    std::thread::spawn(move || {
        pollster::block_on(eid_core::run(Pcsc::new(), command_rx, |s| {
            status_tx.send(s).ok();
        }))
    });
    let doc = loop {
        match statuses.recv().map_err(|_| "the reader watcher stopped")? {
            Event::Readers(readers) => {
                let names: Vec<_> = readers.iter().map(|r| r.name.as_str()).collect();
                if let Some(wanted) = &reader
                    && !names.contains(&wanted.as_str())
                {
                    return Err(format!(
                        "no reader named \"{wanted}\"; readers: {}",
                        names.join(", ")
                    ));
                }
                eprintln!("Readers: {}", names.join(", "));
            }
            Event::Status(Status::NoReader) => eprintln!("Waiting for a card reader…"),
            Event::Status(Status::NoCard { reader }) => eprintln!("{reader}: waiting for a card…"),
            Event::Status(Status::Reading { reader }) => eprintln!("{reader}: reading…"),
            Event::Status(Status::Ready { document, .. }) => break document,
            Event::Status(Status::Failed { error, .. }) => return Err(error.to_string()),
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

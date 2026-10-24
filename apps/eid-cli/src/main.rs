//! `eid` — read the card in the first reader that holds one and print it.
//!
//! Usage: `eid [--pdf out.pdf] [--photo out.jpg]`

use std::process::ExitCode;

use eid_core::{Platform, read_eid};
use eid_pcsc::Pcsc;

fn main() -> ExitCode {
    match pollster::block_on(read()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eid: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn read() -> Result<(), String> {
    let (mut pdf_path, mut photo_path) = (None, None);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--pdf", Some(path)) => pdf_path = Some(path),
            ("--photo", Some(path)) => photo_path = Some(path),
            _ => return Err("usage: eid [--pdf out.pdf] [--photo out.jpg]".into()),
        }
    }

    let mut pcsc = Pcsc::new();
    let readers = pcsc.readers(0).await.map_err(|e| e.to_string())?;
    let reader = readers.iter().find(|r| r.has_card).ok_or("no card in any reader")?;
    let mut card = pcsc.connect(&reader.name).await.map_err(|e| e.to_string())?;
    let doc = read_eid(&mut card).await.map_err(|e| e.to_string())?;

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

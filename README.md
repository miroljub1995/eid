# eid

Reads the Serbian electronic ID card (lična karta) and shows the holder's data, and saves the same PDF
print-out as the official "Čitač elektronske lične karte". A Rust port of
[jfreesteel's eidviewer](https://github.com/grakic/jfreesteel/tree/eidviewer-1.1), with the newer card
types from [Baš Čelik](https://github.com/ubavic/bas-celik).

One codebase, egui for the interface. Desktop builds read cards through the system smart card service
(PC/SC), and the browser build talks to the USB reader directly through WebUSB.

Supported cards: ID cards issued before 18 Aug 2014 (Apollo) and after (Gemalto and newer), ID cards for
foreigners, and residence permits.

## Running

```bash
cargo run -p eid-viewer
```

```bash
cargo run -p eid-cli -- --pdf lična-karta.pdf
```

The CLI waits for a card, prints the data and can save the PDF (`--pdf`) and the photo (`--photo`).

**Browser.** Needs a Chromium browser (Chrome, Edge), because they are the only ones with WebUSB. Build
with [trunk](https://trunkrs.dev):

```bash
cd apps/eid-viewer-web && trunk build --release
```

The page is written to `target/web-dist`. Serve it over HTTPS or from localhost, open it, then click
**Poveži čitač** and pick the reader.

Chrome treats smart card readers as a protected USB class: an ordinary web page may list a reader but
not claim it. The app then says so plainly. The browser build works as an
[Isolated Web App](https://github.com/WICG/isolated-web-apps), which can be granted the
`usb-unrestricted` permission. It also needs a machine where the operating system's smart card
service isn't holding the reader. Everywhere else, use the desktop app.

## Layout

```text
crates/
  eid-core     protocol and data model: APDUs, file reading, TLV parsing, the official field order,
               and the SmartCard / Platform traits and the reader watcher (run). No I/O.
  eid-pcsc     Platform over PC/SC: macOS, Windows, Linux (pcsc-lite)
  eid-webusb   Platform over WebUSB: CCID messages straight to a USB reader (wasm only)
  eid-pdf      the A4 print-out (krilla, pure Rust, also works in the browser)
  eid-ui       the egui interface; the platform comes in through the Host trait and a status channel
apps/
  eid-viewer       desktop app (eframe + PC/SC)
  eid-viewer-web   browser app (eframe on wasm + WebUSB), built with trunk
  eid-cli          terminal reader, also the quickest way to test a reader
```

Dependencies only point down the list: `eid-core` knows nothing about readers, PDF or UI, and `eid-ui`
knows nothing about PC/SC or WebUSB.

### Reaching the reader

Every platform reaches the reader differently, so `eid-core` defines two traits and leaves that part to a
backend:

```rust
pub trait SmartCard {
    fn atr(&self) -> &[u8];
    fn transmit(&mut self, apdu: &[u8]) -> impl Future<Output = Result<Vec<u8>, Error>>;
}

pub trait Platform {
    type Card: SmartCard;
    fn readers(&mut self, wait_ms: u32) -> impl Future<Output = Result<Vec<Reader>, Error>>;
    fn connect(&mut self, reader: &str) -> impl Future<Output = Result<Self::Card, Error>>;
    fn sleep(&mut self, ms: u32) -> impl Future<Output = ()>;
}
```

They are async because WebUSB is. A blocking backend like PC/SC just does its work inside the future and
runs on its own thread under `pollster`. `eid_core::run(platform, commands, emit)` watches the readers
and reads a card when one is inserted. It reports a `Status` (`NoReader`, `NoCard`, `Reading`, `Ready`,
`Failed`), which the UI shows. All the card logic, like application selection, T=0 `61xx`/`6Cxx`
follow-ups and file headers, lives in core, so a backend only moves bytes.

To add a platform, implement the two traits in a new crate and start `run` from the app:

| Platform | How to reach the reader |
|---|---|
| macOS, Windows, Linux | `eid-pcsc` (done) |
| Browser | `eid-webusb` (done; CCID, APDU-level readers) |
| Linux/Windows without PC/SC | raw USB with `nusb`: the CCID codec in `eid-webusb/src/ccid.rs` can be reused |
| Android | USB host API (CCID over bulk endpoints, the same codec) through JNI |
| iPadOS | CryptoTokenKit `TKSmartCard` with a USB-C reader |

## Development

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets
```

```bash
cargo run -p eid-ui --example snapshot -- target/snapshots
```

The snapshot example renders every screen offscreen, in light, dark and narrow layouts, with a made-up
holder. Use it to check UI changes without a card.

Non-test code never panics on card data: the workspace lints warn on `unwrap`, `expect`, `panic` and
indexing, and `unsafe` is forbidden.

## The PDF

The print-out replicates the official reader's, which Windows lays out with GDI at 600 dpi. Every rule,
box and baseline uses the original's coordinates. Each glyph sits where GDI put it: its width is
hinted (TrueType, monochrome) at the 600 dpi size and rounded to whole device pixels. Liberation Sans
hints to the same widths as Arial, so the positions are right on every platform, the browser included.
For the glyph shapes, the PDF uses the system's own Arial where there is one (macOS, Windows,
msttcorefonts); it is read at run time and never shipped. Otherwise it uses Liberation Sans.

Compared against an official print of the same card:
- **At 96 dpi:** 2 pixels differ noticeably.
- **At 150 to 600 dpi:** 0.3–0.6% of pixels differ, all antialiasing on glyph edges; every glyph is
  within 0.06 pt.
- **The photo:** it is the card's own JPEG; the official reader re-encodes it.

## Licences

The code is MIT. Liberation Sans (`assets/fonts`) is under the SIL Open Font License 1.1. It is
metric-compatible with Arial, the font of the official print-out, and covers Serbian Cyrillic and
Latin.

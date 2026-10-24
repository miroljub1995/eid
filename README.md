# eid

Reads the Serbian electronic ID card (lična karta) and shows the holder's data, and saves the same PDF
print-out as the official "Čitač elektronske lične karte". A Rust port of
[jfreesteel's eidviewer](https://github.com/grakic/jfreesteel/tree/eidviewer-1.1), with the newer card
types from [Baš Čelik](https://github.com/ubavic/bas-celik).

One codebase, egui for the interface. The desktop app reads cards through the system smart card service
(PC/SC) on macOS, Windows and Linux. The iOS/iPadOS app reads USB-C (or Lightning) readers through
Apple's CryptoTokenKit. The Android app reads USB readers through Android's USB host API and talks
the readers' own protocol (CCID) to them.

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
With several readers attached, `--reader NAME` picks one; the app has a reader picker in its header.

**iPad / iPhone.** It needs iOS/iPadOS 16 or later. Building needs Xcode, an Apple development signing
identity and a provisioning profile for your bundle id:

```bash
rustup target add aarch64-apple-ios
```

```bash
BUNDLE_ID=rs.example.eid SIGN_IDENTITY="Apple Development: …" PROFILE=eid.mobileprovision apps/eid-ios/bundle.sh
```

Install the signed `eID.app` from Xcode's Devices window. Saved PDFs go to the app's folder in the
Files app. The simulator can't reach USB readers, so test on a device.

**Android.** Needs Android 8 or later, a phone or tablet with USB host (USB OTG) support, and a reader
that exchanges whole APDUs (most current readers do). The APK builds in Docker, with nothing to install
but Docker:

```bash
packaging/android/build.sh
```

The script builds an image with Rust, the Android SDK and NDK, and
[cargo-apk](https://crates.io/crates/cargo-apk); building it accepts the Android SDK license. It then
writes `target/apk/release/apk/eid.apk`. The image is amd64 because Google ships the NDK for x86_64
Linux only; on Apple silicon it runs emulated, so the first build is slow. The APK is signed with a
debug key: fine for installing with `adb install`, not for Google Play.

When a reader is plugged in, Android asks once whether eID may use it. Saved PDFs go to
`Android/data/rs.eid.viewer/files`; on Android 11 and later the Files app hides that folder, so copy
them off over USB. Errors are logged to logcat (`adb logcat -s eid`).

There is no browser build: browsers don't let web pages use smart card readers (Chrome counts them as a
protected USB class).

## Layout

```text
crates/
  eid-core     protocol and data model: APDUs, file reading, TLV parsing, the official field order,
               and the SmartCard / Platform traits and the reader watcher (run). No I/O.
  eid-pcsc     Platform over PC/SC: macOS, Windows, Linux (pcsc-lite)
  eid-ctk      Platform over CryptoTokenKit: iOS/iPadOS (and macOS); `unsafe` for the Objective-C calls
  eid-ccid     the USB CCID reader protocol over any pair of bulk endpoints
  eid-android-usb  Platform over Android's USB host API (JNI) + eid-ccid
  eid-pdf      the A4 print-out (krilla, pure Rust)
  eid-ui       the egui interface (egui only; each app picks its own renderer); the platform comes in
               through the Host trait and an event channel
apps/
  eid-viewer       desktop app (eframe on OpenGL + PC/SC)
  eid-ios          iOS/iPadOS app (eframe on Metal + CryptoTokenKit)
  eid-android      Android app (eframe on OpenGL ES + USB host), built with cargo-apk
  eid-cli          terminal reader, also the quickest way to test a reader
```

Dependencies only point down the list: `eid-core` knows nothing about readers, PDF or UI, and `eid-ui`
knows nothing about PC/SC.

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

They are async so that backends built on callback or async APIs (Android, iPadOS) fit. A blocking backend like PC/SC just does its work inside the future and
runs on its own thread under `pollster`. `eid_core::run(platform, commands, emit)` watches the readers
and reads a card when one is inserted, from the reader picked with `Command::Select` or else the first
one holding a card. It reports the reader list and a `Status` (`NoReader`, `NoCard`, `Reading`, `Ready`,
`Failed`), which the UI shows. All the card logic, like application selection, T=0 `61xx`/`6Cxx`
follow-ups and file headers, lives in core, so a backend only moves bytes.

To add a platform, implement the two traits in a new crate and start `run` from the app:

| Platform | How to reach the reader |
|---|---|
| macOS, Windows, Linux | `eid-pcsc` (done) |
| Android | `eid-android-usb` (done; USB host API + CCID, APDU-level readers) |
| Linux/Windows without PC/SC | raw USB (e.g. `nusb`) + `eid-ccid` |
| iOS, iPadOS | `eid-ctk` (done; CryptoTokenKit, iOS 16+) |

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
indexing, and `unsafe` is forbidden everywhere but the platform glue: `eid-ctk` (Objective-C),
`eid-android-usb` (adopting the JVM pointers) and the Android app's exported entry point.

To test the CryptoTokenKit backend on a Mac, sign the binary with the
`com.apple.security.smartcard` entitlement; on macOS, CryptoTokenKit is only reachable with it.

## The PDF

The print-out replicates the official reader's, which Windows lays out with GDI at 600 dpi. Every rule,
box and baseline uses the original's coordinates. Each glyph sits where GDI put it: its width is
hinted (TrueType, monochrome) at the 600 dpi size and rounded to whole device pixels. Liberation Sans
hints to the same widths as Arial, so the positions are right on every platform.
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

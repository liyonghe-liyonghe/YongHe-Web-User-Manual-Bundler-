# Yonghe — Web Page / User Manual Synthesizer

Synthesizes web pages, images, documents, and other files — 23 formats in total — into a single structured, hierarchically navigable HTML manual.
All resources (images, styles, scripts) are fully embedded; both phones and PCs can read it completely offline through the browser they already have.
Supports 13 interface languages; currently Windows only, Linux planned. Built on Tauri v2 + Rust, MIT licensed.

<img width="1102" height="792" alt="interface" src="https://github.com/user-attachments/assets/a64f171e-0efc-4990-8740-2c2d886a3cb3" />
<img width="1508" height="437" alt="image-2" src="https://github.com/user-attachments/assets/b453ac82-eab1-4569-9264-9547720e88db" />
<img width="1508" height="437" alt="image-3" src="https://github.com/user-attachments/assets/56f66c84-e62e-4d45-89cf-ce1dc2ee9e50" />



- Version: `0.8.7`
- License: MIT (see `LICENSE`)
- First public tag: `v0.8.7`
- Repository URL: TBD (to be filled in by the publisher after creating the repository; this document does not preset any unconfirmed address)

## Features

- **Unify documents of different formats into a single structured, hierarchically navigable HTML manual**. After exporting the HTML, view it offline on phones or PCs using the browsers already installed everywhere — no need to install various software.
  Supported formats:
  Web pages: .html .htm .mht .mhtml
  Images: .png .jpg .jpeg .gif .webp .bmp .svg .ico
  Documents: .pdf .doc .docx .rtf .wps .odt .odp .ppt .pptx .txt .md; PDFs are rendered page-by-page with full fidelity, falling back to text extraction when the dynamic library is missing.
- **Tolerant multi-encoding decoding**: 19 encoding candidates combined with Han/Kana/Hangul gating automatically restores documents in legacy encodings (GBK/Big5/Shift-JIS, etc.).
- **Visual navigation structure editing**: feed the table of contents in the left pane, drag to group/reorder/rename in the middle pane, live preview in the right pane.
- **Automatic missing-asset completion**: when local resources referenced by a page are missing, the app automatically searches recorded directories to fill them in, with a fallback dialog to choose a source folder.
- **Single-file offline distribution**: embeds all assets plus http(s) external images; the exported HTML works offline.
- **Multilingual interface**: 13 interface languages; the UI and incremental auxiliary controls are multilingual, while the language of page content is determined by the source material — the program does not translate body text. All 13 language packs are embedded in `src/i18n.js` (to avoid fetch being blocked under `file://`); `src/locales/zh-CN.json` is a synchronized copy of the Chinese baseline pack and is not loaded at runtime.
- **Parallel processing**: rayon parallelizes page conversion and media decoding across CPU cores.

## Running

Option 1: Download the executable from the Release attachments and run it directly (portable build, no installation required).

Option 2: Build from source and run (see the next section).

## Building from Source

Prerequisites:

- Rust (edition 2021; Tauri 2.x)
- Tauri v2 desktop dependencies (Windows requires WebView2 and MSVC build tools)
- Node.js (only for npm script entry points; the frontend is vanilla JS with no framework build step)

Steps:

```powershell
cd src-tauri
cargo build --release
```

Run in development mode:

```powershell
cargo tauri dev
```

> `src-tauri/resources/pdfium.dll` is the dynamic library for PDF fidelity rendering and must be deployed in the same directory as the executable; its license is in `src-tauri/resources/pdfium-LICENSE.txt`.

## Testing

```powershell
cd src-tauri
cargo test
```

There are currently 50 `#[test]` cases (anchor list in `发布准备-现状锚点登记-20261121.md`).

## Directory Structure

```
src/                 Frontend (vanilla JS/HTML/CSS, i18n with embedded language packs)
src-tauri/           Rust backend (Tauri v2)
  src/               Conversion, packaging, MHTML, command layer
  examples/          Local test and matrix audit tools (not shipped with the library)
  resources/         pdfium.dll and its license
  capabilities/      Desktop permission configuration
  icons/             Application icons
_tools/              Test asset generation scripts (development aid, not shipped with the library)
```

For the ship whitelist and exclusion list, see the preparation documents such as `发布准备-随库白名单清单-20261121.md`.

## License

This project is MIT licensed; see `LICENSE` for details. The third-party component pdfium is BSD-3-Clause licensed; the full text is in `src-tauri/resources/pdfium-LICENSE.txt`.

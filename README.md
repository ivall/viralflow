<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="88" alt="">

# ViralFlow

Free, open-source alternative to CapCut.<br>
Lightweight & fast.

[![CI](https://github.com/ivall/viralflow/actions/workflows/ci.yml/badge.svg)](https://github.com/ivall/viralflow/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ivall/viralflow?include_prereleases&sort=semver)](https://github.com/ivall/viralflow/releases)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-informational)
<br>
![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8D8?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust)
![Vue 3](https://img.shields.io/badge/Vue-3-42b883?logo=vuedotjs&logoColor=white)
![FFmpeg](https://img.shields.io/badge/FFmpeg-required-007808?logo=ffmpeg&logoColor=white)

<img src="docs/editor.webp" alt="ViralFlow editor with a vertical project, text styles open, a photo overlay selected on the canvas and word-highlight captions on the timeline">

</div>

## Features

- Multi-track timeline with video, photos, text, stickers and audio.
- Move, scale and rotate anything right on the preview, with keyframes for animation.
- Auto captions with word-by-word timing, generated locally with Whisper, with styles that light up each spoken word.
- Transitions and effects you drag onto the timeline and stretch to length, plus in/out animations and color correction.
- Text styles made for short-form video, with custom fonts, outlines, boxes, glows and shadows.
- Export to MP4, MOV or WebM up to 4K, with optional GPU encoding.
- Optional AI tools through your own OpenRouter key.

<table>
  <tr>
    <td width="40%"><img src="docs/captions.webp" alt="Captions panel and a caption with the spoken word highlighted"></td>
    <td width="40%"><img src="docs/effects.webp" alt="Effects panel, color adjustments and effects placed on clips in the timeline"></td>
    <td width="20%"><img src="docs/export-preview.webp" alt="Exported 9:16 video with a highlighted caption"></td>
  </tr>
  <tr>
    <td align="center"><sub>Captions</sub></td>
    <td align="center"><sub>Effects and color</sub></td>
    <td align="center"><sub>Exported result</sub></td>
  </tr>
</table>

## Install

Grab the latest build from [Releases](https://github.com/ivall/viralflow/releases):
`.exe` / `.msi` for Windows, `.AppImage` / `.deb` / `.rpm` for Linux.

The app needs [FFmpeg](https://ffmpeg.org) with `ffprobe`:

```sh
winget install Gyan.FFmpeg     # Windows (the full build includes Whisper for local captions)
sudo apt install ffmpeg        # Debian/Ubuntu (the .deb and .rpm pull it in automatically)
```

If FFmpeg isn't on `PATH`, point to its folder in **Settings → Tools**. Builds aren't code-signed yet, so Windows SmartScreen may ask for confirmation on first launch.

## Build from source

Requires Node.js 22+, stable Rust and FFmpeg. On Linux also the [Tauri system packages](https://tauri.app/start/prerequisites/#linux).

```sh
npm install
npm run app:dev       # run with hot reload
npm run app:build     # installers in src-tauri/target/release/bundle
```

Checks, same as CI:

```sh
npm run typecheck && npm test
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## License

[GPL-3.0](LICENSE). Bundled fonts are under the SIL Open Font License, see [`public/licenses`](public/licenses/FONTS.md).

# Architecture

ViralFlow is a Rust workspace: a Slint application on top of five libraries that know
nothing about the UI.

```
app (Slint UI, editor controller, export, settings)
 ├─ vf-ai      Whisper captions, transcripts, OpenRouter client
 ├─ vf-audio   playback mixer and the export soundtrack
 ├─ vf-render  wgpu compositor, text rasteriser, preview frame cache
 ├─ vf-media   libav: probe, decode (CPU/GPU), filters, encode, background jobs
 └─ vf-core    project model, editing with undo, scene engine, timeline rules (no I/O)
```

## Project model and editing (`vf-core`)

- `model.rs` is the document: tracks of timeline items, media assets, export settings. Every
  type is `#[serde(default)]` and keeps fields it does not know in `extra`, so a project saved
  by a newer version loses nothing when an older one opens it.
- `migrate.rs` reads any stored document: version 1 (the old Tauri app) is converted, newer
  schema versions are refused, and a field with a wrong value is dropped on its own (its default
  applies) instead of costing the whole project.
- `edit.rs` (`Editor`) is the only way the app changes a project. It keeps tracks free of
  overlaps, transitions on real cuts and keyframes inside their clips. Undo steps are snapshots
  that share unchanged tracks and assets with their neighbours (`Arc`).
- `engine/` resolves what is on screen at a moment (`resolve_scene`): transforms, keyframes,
  animations, effects and transitions. Preview and export both use it.
- `time.rs` has exact time helpers: frame indices at the project rate and stream time bases
  as fractions.

## Pictures (`vf-media`, `vf-render`)

- `vf-media::video::VideoDecoder` returns "the frame on screen at time t". Heavy formats (HEVC,
  AV1, VP9, large or 10-bit H.264) decode on the GPU (D3D11VA, VideoToolbox; VAAPI opt-in) and
  fall back to the CPU from the frame where the GPU failed. A keyframes-only mode gives
  filmstrips and thumbnails one decoded picture each.
- `vf-render::FrameRenderer` composites a frame with wgpu. In `Mode::Export` every picture is
  exact and fetched inline; in `Mode::Preview` pictures come from background workers and the UI
  never waits.
- `vf-render/src/frames.rs` is the preview's frame cache: one worker per visible clip decodes
  the frame under the playhead first, then a second ahead while playing; frames are shared by
  every clip of a file and bounded by a share of the machine's memory.
- The preview texture goes straight to Slint (`slint::Image::try_from(wgpu::Texture)`), with
  no copy back to the CPU.
- `vf-media::jobs` is a small priority pool for background media work (timeline filmstrips and
  waveforms before library thumbnails).

## Sound (`vf-audio`)

- `voice.rs` is one clip's sound: decoded, sped up or slowed down without changing the pitch
  (`atempo`), cleaned (`afftdn`, de-esser, compressor) with the filters' delay compensated.
- `mixer.rs` (`Mix`) renders any stretch of the timeline from those voices. The preview's mixer
  thread plays it into a lock-free ring; the export renders it from start to end, so both hear
  the same samples.
- `loudness.rs` measures integrated loudness (BS.1770, checked against FFmpeg's `ebur128`) and
  is the master stage both share: one gain to the target, then a peak limiter.
- `lib.rs` (`Player`) owns the cpal stream and rebuilds it when the output device changes.

## Captions and AI (`vf-ai`)

- `whisper.rs` runs whisper.cpp through FFmpeg's `whisper` filter, in process.
- `transcript.rs` turns segments and word timings into caption cues; word-highlight captions
  are one clip per word (`vf_core::captions::highlight_cues`), timed by the transcript or
  estimated from the words' length.
- `openrouter.rs` is the optional cloud client (text rewrite, cloud transcription, photo
  matching) with the user's own key, kept in the system credential store.

## The app (`app`)

- `editor/` is the editor controller: one `Session` per open project (editor, preview, audio
  player, timeline models), the 60 fps tick, autosave and every panel's bridge to Slint.
- `ui/` holds the Slint files. Repeaters are fed persistent `VecModel`s updated in place
  (`models::sync_rows`) so pointer grabs survive updates.
- `ui_tests.rs` drives the real UI headless (Slint's testing backend): clicks, drags,
  shortcuts, export to a file.

## Packaging

- Windows: `scripts/package.ps1` (portable .zip, Inno Setup installer).
- Linux: `scripts/package-linux.sh` (AppImage and .tar.gz; FFmpeg in `lib/`, found via rpath).
- macOS: `scripts/package-macos.sh` (.app with FFmpeg in `Frameworks/`, .dmg and .zip).
- `.github/workflows/release.yml` builds all of them for a version tag and drafts a release.

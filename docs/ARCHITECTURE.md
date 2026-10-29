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
- A clip carries a stack of effects (`ClipEffect`: kind, strength 0..1, optional time range);
  `engine/effects.rs` turns each into an `EffectState` and adds them up. What the compositor
  does beyond moving and recolouring a layer (pixel blocks, RGB split, lens, mirror, zoom blur,
  light leak, vertical wipes, iris and clock masks) is a `LayerFx` (`engine/fx.rs`), produced
  by effects and transitions alike. Documents from before effect stacks have one `effect`,
  which `migrate.rs` moves into the stack.
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

- `whisper.rs` runs whisper.cpp in process (whisper-rs; Metal on macOS) on the clip's sound at
  16 kHz mono (`speech.rs`, decoded a second at a time). Word timings come from the token
  timestamps of the same pass. Models download from a pinned Hugging Face commit, resume after an
  interruption and are checked against their SHA-256 before they are used.
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
- `editor/cards.rs` draws the effect and transition cards: a small landscape through the real
  scene engine and compositor, so a card shows exactly what the export will.
- `logs.rs` writes each run's log to a file (the last ten are kept, each capped) and logs
  panics with a backtrace; a panic on the main thread ends with a dialog naming the file.
- `ui_tests.rs` drives the real UI headless (Slint's testing backend): clicks, drags,
  shortcuts, export to a file.

## The AI editor (`app/src/agent`)

- `specs.rs` lists every tool with its JSON Schema; the chat (OpenAI function calling) and MCP use the same list.
- `edits.rs` holds the editing tools: pure changes through `Editor`, validated with errors a model can act on,
  reporting each clip as it ends up. `describe.rs` is the compact JSON the model reads (timeline times, defaults left
  out, caption groups summarised).
- `editor/agent.rs` adds what needs more than the document: rendering frames (export renderer, own GPU device),
  frames of files, speech recognition (cached per file), beats, import, playback and export. Each tool call is one
  undo step (`Editor::begin_batch`).
- Tools run on the UI thread (`bridge.rs`); callers wait on their own thread. Long work (`Reply::Later`, `jobs.rs`)
  never blocks the UI: a tool waits a while for its job and otherwise returns its id for `job_status`.
- `chat.rs` is the AI panel: the tool-calling loop against OpenRouter on a background thread, one conversation per
  project (`assistant.json` next to the project), "undo this request" from the document saved before it.
- `mcp.rs` serves MCP over HTTP on 127.0.0.1 (token, Host/Origin checks) while the app runs; `viralflow --mcp` is the
  stdio server AI apps start: it answers the handshake itself and forwards tool calls, starting the app if needed.

## Packaging

- Windows: `scripts/package.ps1` (portable .zip, Inno Setup installer).
- Linux: `scripts/package-linux.sh` (AppImage and .tar.gz; FFmpeg in `lib/`, found via rpath).
- macOS: `scripts/package-macos.sh` (.app with FFmpeg in `Frameworks/`, .dmg and .zip).
- `.github/workflows/release.yml` builds all of them for a version tag and drafts a release.

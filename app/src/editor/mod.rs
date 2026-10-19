//! The editor page: one open project, its preview, playback and autosave.

mod assets;
mod canvas;
mod captions;
mod cards;
mod export_ui;
mod inspector;
mod overview;
mod preview;
mod timeline;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, VecModel};
use vf_core::edit::{Editor, Flag, Placement};
use vf_core::engine::scene::layer_quad;
use vf_core::model::MediaAsset;
use vf_core::presets::{aspect_label, ASPECT_PRESETS};
use vf_core::timeline::Edge;
use vf_render::TextEngine;

use crate::app::Context;
use crate::{
    format, media, projects, AssetsState, CaptionsState, DragState, EditorState, ExportState, Handle, InspectorState, Page, PointerPhase, SaveState,
    TimelineState,
};
use canvas::{Gesture, Mode};
use preview::Preview;
use timeline::{Request, TimelineView};

/// Autosave runs this long after the last change...
const SAVE_DELAY: Duration = Duration::from_millis(700);
/// ...or at the latest this long after the first unsaved one (long drags).
const MAX_SAVE_DELAY: Duration = Duration::from_secs(5);

struct Session {
    editor: Editor,
    preview: Preview,
    audio: vf_audio::Player,
    saved_revision: u64,
    /// First change not saved yet.
    changed_at: Option<Instant>,
    /// Most recent change (autosave waits until edits pause).
    last_change: Option<Instant>,
    /// Selection outlines on the canvas (a model that lives with the session).
    handles: std::rc::Rc<VecModel<Handle>>,
    handles_bound: bool,
    save_failed: bool,
    needs_render: bool,
    incomplete: bool,
    gesture: Option<Gesture>,
    /// A press on the selection over other layers: dragging moves the
    /// selection, a plain click selects this top-most layer instead.
    click_pick: Option<String>,
    guides: canvas::Guides,
    muted: bool,
    timeline: TimelineView,
    /// Last state pushed to the inspector.
    inspector_key: Option<inspector::SyncKey>,
    /// Last keystroke in the inspector's text box (groups typing into one undo step).
    typing: Option<Instant>,
    /// Playback stops here (animation previews).
    stop_at: Option<f64>,
    /// A drag from the asset panels.
    drag: Option<assets::Drag>,
    /// Assets whose files are missing.
    offline: std::collections::HashSet<String>,
    assets_key: assets::ListKey,
}

impl Session {
    fn playing(&self) -> bool {
        self.audio.is_playing()
    }
}

pub struct EditorController {
    context: Rc<Context>,
    text: Arc<TextEngine>,
    session: RefCell<Option<Session>>,
    timer: slint::Timer,
    /// The running export (it outlives the editor session that started it).
    export_job: RefCell<Option<crate::export::Job>>,
    export_timer: slint::Timer,
    export_path: RefCell<PathBuf>,
    export_result: RefCell<String>,
    quit_after_export: RefCell<bool>,
    captions: RefCell<captions::Panel>,
    /// Effect and transition cards (drawn on first use).
    cards: RefCell<Option<cards::Cards>>,
    cards_timer: slint::Timer,
    /// Called after the editor closed (the project list should refresh).
    pub on_closed: RefCell<Option<Box<dyn Fn()>>>,
}

impl EditorController {
    pub fn new(context: Rc<Context>, text: Arc<TextEngine>) -> Rc<Self> {
        let controller = Rc::new(EditorController {
            context: context.clone(),
            text,
            session: RefCell::default(),
            timer: slint::Timer::default(),
            export_job: RefCell::default(),
            export_timer: slint::Timer::default(),
            export_path: RefCell::default(),
            export_result: RefCell::default(),
            quit_after_export: RefCell::default(),
            captions: RefCell::new(captions::Panel::new(&context.settings.get().caption_language)),
            cards: RefCell::default(),
            cards_timer: slint::Timer::default(),
            on_closed: RefCell::default(),
        });
        if let Some(window) = context.window() {
            controller.bind(&window.global::<EditorState>());
        }
        controller
    }

    fn bind(self: &Rc<Self>, state: &EditorState) {
        macro_rules! on {
            ($method:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                state.$method(move |$($arg),*| {
                    if let Some($this) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(on_back, |this| this.close());
        on!(on_rename, |this, name| this.edit(|e| e.rename(&name)));
        on!(on_export, |this| this.open_export());
        on!(on_more, |this, id| this.more(&id));
        on!(on_preview_size, |this, width, height| this.resize_preview(width, height));
        on!(on_canvas_pointer, |this, phase, x, y, shift, control| this.canvas_pointer(phase, x as f64, y as f64, shift, control));
        on!(on_canvas_hover, |this, x, y| this.canvas_hover(x as f64, y as f64));
        on!(on_toggle_play, |this| this.toggle_play());
        on!(on_seek, |this, time| this.seek(time as f64));
        on!(on_step, |this, frames| this.step(frames));
        on!(on_go_start, |this| this.seek(0.0));
        on!(on_go_end, |this| {
            let end = this.with(|s| s.editor.playback_end()).unwrap_or(0.0);
            this.seek(end)
        });
        on!(on_set_aspect, |this, id| {
            if let Some(preset) = ASPECT_PRESETS.iter().find(|p| p.id == id.as_str()) {
                this.edit(|e| e.set_canvas(preset.width, preset.height));
            }
        });
        on!(on_toggle_mute, |this| {
            this.with(|s| {
                s.muted = !s.muted;
                s.audio.set_master(if s.muted { 0.0 } else { 1.0 });
            });
            this.sync_ui();
        });
        on!(on_toggle_fullscreen, |this| this.toggle_fullscreen());
        on!(on_import_media, |this| this.import_media(true));
        on!(on_panel_changed, |this, panel| {
            if panel == "captions" {
                this.sync_captions();
            }
        });
        let weak = Rc::downgrade(self);
        state.on_key(move |text, control, shift, alt| weak.upgrade().is_some_and(|this| this.key(&text, control, shift, alt)));
        if let Some(window) = self.context.window() {
            self.bind_timeline(&window.global::<TimelineState>());
            self.bind_inspector(&window.global::<InspectorState>());
            self.bind_assets(&window.global::<AssetsState>(), &window.global::<DragState>());
            self.install_file_drop();
            self.bind_export(&window.global::<ExportState>());
            self.bind_captions(&window.global::<CaptionsState>());
        }
    }

    fn bind_timeline(self: &Rc<Self>, state: &TimelineState) {
        macro_rules! on {
            ($method:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                state.$method(move |$($arg),*| {
                    if let Some($this) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }
        on!(on_action, |this, action| this.timeline_action(&action));
        on!(on_zoom_to, |this, value| {
            this.with(|s| s.timeline.set_zoom((value as f64).exp(), None, &s.editor));
            this.sync_ui();
        });
        on!(on_lanes_size, |this, width, height| {
            this.with(|s| s.timeline.set_size(width as f64, height as f64, &s.editor));
            this.sync_ui();
        });
        on!(on_lanes_pointer, |this, phase, x, y, shift, control, _alt| {
            let mut requests = Vec::new();
            let changed = this.with(|s| {
                let playing = s.playing();
                let before = s.editor.revision();
                if phase == PointerPhase::Down && playing {
                    s.audio.stop();
                }
                s.timeline.lanes_pointer(&mut s.editor, phase, x as f64, y as f64, shift, control, playing, &mut requests);
                s.needs_render = true;
                s.editor.revision() != before
            });
            this.after_timeline(changed, requests);
        });
        on!(on_lanes_hover, |this, x, y| {
            this.with(|s| s.timeline.hover(&s.editor, x as f64, y as f64));
            this.sync_timeline();
        });
        on!(on_lanes_context, |this, x, y| {
            let entries = this.with(|s| s.timeline.context(&mut s.editor, x as f64, y as f64)).unwrap_or_default();
            if let Some(window) = this.context.window() {
                window.global::<TimelineState>().set_context_entries(ModelRc::new(VecModel::from(entries)));
            }
            this.sync_ui();
        });
        on!(on_context_action, |this, action| {
            let mut requests = Vec::new();
            let changed = this.with(|s| {
                let before = s.editor.revision();
                s.timeline.context_action(&mut s.editor, &action, &mut requests);
                s.editor.revision() != before
            });
            this.after_timeline(changed, requests);
        });
        on!(on_ruler_pointer, |this, phase, x, shift| {
            this.with(|s| {
                if phase == PointerPhase::Down && s.playing() {
                    s.audio.stop();
                }
                s.timeline.ruler_pointer(&mut s.editor, phase, x as f64, shift);
                s.needs_render = true;
            });
            this.sync_ui();
        });
        on!(on_scroll, |this, dx, dy, control, shift, x| {
            this.with(|s| s.timeline.scroll(dx as f64, dy as f64, control, shift, x as f64, &s.editor));
            this.sync_timeline();
        });
        on!(on_hscroll, |this, pos, smooth| {
            this.with(|s| s.timeline.scroll_to_fraction(pos as f64, smooth, &s.editor));
            this.sync_timeline();
        });
        on!(on_header_pointer, |this, phase, id, y| {
            let changed = this.with(|s| {
                let before = s.editor.revision();
                s.timeline.header_pointer(&mut s.editor, phase, &id, y as f64);
                s.editor.revision() != before
            });
            this.after_timeline(changed, vec![]);
        });
        on!(on_track_toggle, |this, id, flag| {
            this.edit(|e| {
                let Some(track) = e.project.tracks.iter().find(|t| t.id == id.as_str()) else { return };
                let mut patch = vf_core::edit::TrackPatch::default();
                match flag.as_str() {
                    "hidden" => patch.hidden = Some(!track.hidden),
                    "muted" => patch.muted = Some(!track.muted),
                    _ => patch.locked = Some(!track.locked),
                }
                e.update_track(&id, patch);
            });
        });
        on!(on_track_action, |this, id, action| {
            this.edit(|e| {
                let Some(index) = e.project.tracks.iter().position(|t| t.id == id.as_str()) else { return };
                match action.as_str() {
                    "up" if index > 0 => e.move_track(&id, index - 1),
                    "down" => e.move_track(&id, index + 1),
                    "delete" => e.remove_track(&id),
                    _ => {}
                }
            });
        });
        on!(on_rename_track, |this, id, name| this
            .edit(|e| e.update_track(&id, vf_core::edit::TrackPatch { name: Some(name.to_string()), ..Default::default() })));
        on!(on_add_track, |this, kind| {
            let kind = vf_core::model::TrackKind::parse(&kind).unwrap_or_default();
            this.edit(|e| e.add_track(kind));
        });
    }

    fn after_timeline(&self, changed: Option<bool>, requests: Vec<Request>) {
        if changed == Some(true) {
            self.changed();
        } else {
            self.sync_ui();
        }
        for request in requests {
            match request {
                Request::OpenPanel(panel) => self.open_panel(panel),
            }
        }
    }

    /// Shows an asset panel from code (the Slint tabs report their own clicks).
    fn open_panel(&self, panel: &str) {
        if let Some(window) = self.context.window() {
            window.global::<EditorState>().set_panel(panel.into());
        }
        // Refresh it right away instead of relying on Slint's change notification.
        if panel == "captions" {
            self.sync_captions();
        }
    }

    fn timeline_action(&self, action: &str) {
        match action {
            "undo" => self.edit(|e| {
                e.undo();
            }),
            "redo" => self.edit(|e| {
                e.redo();
            }),
            "split" => self.edit(|e| {
                let time = e.time;
                e.split_at(time)
            }),
            "trim-start" => self.edit(|e| e.trim_to_playhead(Edge::Start)),
            "trim-end" => self.edit(|e| e.trim_to_playhead(Edge::End)),
            "duplicate" => self.edit(|e| e.duplicate_selected()),
            "delete" => self.edit(|e| e.delete_selected(false)),
            "keyframe" => self.edit(|e| e.toggle_keyframe_at_playhead()),
            "transitions" => self.open_panel("transitions"),
            "snap" => {
                self.with(|s| s.timeline.snapping = !s.timeline.snapping);
                self.sync_ui();
            }
            "zoom-in" | "zoom-out" | "fit" => {
                self.with(|s| match action {
                    "zoom-in" => s.timeline.set_zoom(s.timeline.zoom * 1.4, None, &s.editor),
                    "zoom-out" => s.timeline.set_zoom(s.timeline.zoom / 1.4, None, &s.editor),
                    _ => s.timeline.zoom_to_fit(&s.editor),
                });
                self.sync_ui();
            }
            _ => {}
        }
    }

    fn sync_timeline(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<TimelineState>();
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.timeline.sync(&session.editor, &state);
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Session) -> T) -> Option<T> {
        self.session.borrow_mut().as_mut().map(f)
    }

    // ------------------------------------------------------------ lifecycle

    pub fn open(self: &Rc<Self>, id: &str) {
        self.close_session();
        let Some(window) = self.context.window() else { return };
        let state = window.global::<EditorState>();
        state.set_loading(true);
        state.set_load_error("".into());
        self.context.go(Page::Editor);
        match projects::load(&self.context.paths.projects, id) {
            Ok(project) => {
                window.set_window_title(format!("{} - ViralFlow", project.name).into());
                let editor = Editor::new(project);
                let saved_revision = editor.revision();
                let audio = vf_audio::Player::new();
                audio.set_project(Arc::new(editor.project.clone()));
                let mut preview = Preview::new(&self.context.gpu, self.text.clone());
                let size = window.window().size().to_logical(window.window().scale_factor());
                preview.set_viewport(size.width * 0.5, size.height * 0.5, window.window().scale_factor());
                *self.session.borrow_mut() = Some(Session {
                    editor,
                    preview,
                    audio,
                    saved_revision,
                    changed_at: None,
                    last_change: None,
                    handles: Default::default(),
                    handles_bound: false,
                    save_failed: false,
                    needs_render: true,
                    incomplete: false,
                    gesture: None,
                    click_pick: None,
                    guides: Default::default(),
                    muted: false,
                    timeline: TimelineView::new(self.context.settings.get().snap),
                    inspector_key: None,
                    typing: None,
                    stop_at: None,
                    drag: None,
                    offline: Default::default(),
                    assets_key: Default::default(),
                });
                state.set_loading(false);
                self.check_media();
                self.start_timer();
                self.sync_ui();
                self.sync_captions();
            }
            Err(error) => {
                state.set_loading(false);
                state.set_load_error(error.to_string().into());
            }
        }
    }

    /// Development aid (`VF_SELECT`, `VF_TAB`): selects the n-th clip and opens an inspector tab.
    pub fn dev_select(&self, index: usize, tab: Option<String>) {
        self.edit(|e| {
            let Some((id, start)) = e.project.items().nth(index).map(|item| (item.id.clone(), item.start)) else { return };
            e.select(Some(&id), false);
            e.seek(start + 0.5);
        });
        if let (Some(tab), Some(window)) = (tab, self.context.window()) {
            window.global::<InspectorState>().set_tab(tab.into());
        }
    }

    /// The open project (tests check what UI actions did to it).
    #[cfg(test)]
    pub fn project(&self) -> Option<vf_core::model::Project> {
        self.with(|s| s.editor.project.clone())
    }

    #[cfg(test)]
    pub fn with_primary(&self) -> Option<String> {
        self.with(|s| s.editor.primary.clone()).flatten()
    }

    #[cfg(test)]
    pub fn select(&self, id: &str) {
        self.with(|s| s.editor.select(Some(id), false));
        self.sync_ui();
    }

    /// Runs one frame of the editor loop (tests have no event loop).
    #[cfg(test)]
    pub fn step_frame(&self) {
        self.tick();
    }

    fn start_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.timer.start(slint::TimerMode::Repeated, Duration::from_millis(15), move || {
            if let Some(this) = weak.upgrade() {
                this.tick();
            }
        });
    }

    /// Saves, writes the thumbnail and returns to the project list.
    pub fn close(&self) {
        self.close_session();
        if let Some(window) = self.context.window() {
            window.set_window_title("ViralFlow".into());
        }
        self.context.go(Page::Home);
        if let Some(callback) = self.on_closed.borrow().as_ref() {
            callback();
        }
    }

    fn close_session(&self) {
        self.timer.stop();
        let Some(mut session) = self.session.borrow_mut().take() else { return };
        session.audio.stop();
        self.save(&mut session);
        let project = &session.editor.project;
        let has_content = project.items().next().is_some();
        if has_content && !session.save_failed {
            let time = if session.editor.time > 0.0 && session.editor.time < session.editor.playback_end() {
                session.editor.time
            } else {
                (session.editor.playback_end() * 0.2).min(3.0)
            };
            if let Some(jpeg) = session.preview.thumbnail(project, time) {
                let _ = projects::save_thumbnail(&self.context.paths.projects, &project.id, &jpeg);
            }
        }
        // The project's decoded frames are no use on the project list.
        drop(session);
        vf_render::release_preview_frames();
    }

    /// Saves now if there are unsaved changes.
    pub fn flush(&self) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            self.save(session);
        }
    }

    fn save(&self, session: &mut Session) {
        if session.editor.revision() == session.saved_revision && !session.save_failed {
            return;
        }
        match projects::save(&self.context.paths.projects, &session.editor.project) {
            Ok(()) => {
                session.saved_revision = session.editor.revision();
                session.save_failed = false;
                session.changed_at = None;
            }
            Err(error) => {
                if !session.save_failed {
                    self.context.toasts.error("Project could not be saved", &error.to_string());
                }
                session.save_failed = true;
            }
        }
    }

    /// Marks assets whose files are missing.
    fn check_media(&self) {
        let missing: Vec<String> = self
            .with(|s| {
                let missing: Vec<&MediaAsset> = s.editor.project.assets.iter().filter(|a| !std::path::Path::new(&a.path).is_file()).collect();
                let names = missing.iter().map(|a| a.name.clone()).collect();
                s.offline = missing.iter().map(|a| a.id.clone()).collect();
                s.assets_key = Default::default();
                names
            })
            .unwrap_or_default();
        if !missing.is_empty() {
            let detail = if missing.len() == 1 { missing[0].clone() } else { format!("{} and {} more", missing[0], missing.len() - 1) };
            self.context.toasts.error("Some media files are missing", &detail);
        }
    }

    // ------------------------------------------------------------ the frame loop

    fn tick(&self) {
        let mut ui = false;
        let mut timeline = false;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if session.timeline.overviews.poll() {
                ui = true;
            }
            if session.timeline.animate(&session.editor) {
                timeline = true;
            }
            if session.playing() {
                let end = session.editor.playback_end();
                let time = session.audio.time();
                let end = session.stop_at.map_or(end, |stop| stop.min(end));
                if time >= end {
                    session.audio.stop();
                    session.editor.time = end;
                } else {
                    session.editor.time = time;
                }
                session.timeline.follow(session.editor.time, &session.editor);
                session.needs_render = true;
                ui = true;
            }
            let autosave = self.context.settings.get().autosave;
            if session.editor.revision() != session.saved_revision {
                let since = *session.changed_at.get_or_insert_with(Instant::now);
                let paused = session.last_change.is_none_or(|last| last.elapsed() >= SAVE_DELAY);
                if autosave && (paused || since.elapsed() >= MAX_SAVE_DELAY) && !session.save_failed {
                    self.save(session);
                    ui = true;
                }
            }
            session.audio.maintain();
            if session.needs_render || session.incomplete {
                session.needs_render = false;
                let playing = session.playing();
                session.preview.renderer.set_playing(playing);
                let project = &session.editor.project;
                if let Some((image, complete)) = session.preview.render(project, session.editor.time) {
                    session.incomplete = !complete;
                    if let Some(window) = self.context.window() {
                        window.global::<EditorState>().set_frame(image);
                    }
                }
            }
        }
        if ui {
            self.sync_ui();
        } else if timeline {
            self.sync_timeline();
        }
    }

    /// After any change to the project: redraw, restart sound, mark unsaved.
    fn changed(&self) {
        self.with(|s| {
            s.needs_render = true;
            s.changed_at.get_or_insert_with(Instant::now);
            s.last_change = Some(Instant::now());
            // Also while paused: the mixer measures loudness ahead of playback
            // (only when the sound changed).
            s.audio.set_project(Arc::new(s.editor.project.clone()));
        });
        self.sync_ui();
        self.sync_captions();
    }

    /// Runs an edit and refreshes everything that depends on the project.
    fn edit(&self, f: impl FnOnce(&mut Editor)) {
        let changed = self.with(|s| {
            let before = s.editor.revision();
            f(&mut s.editor);
            s.editor.revision() != before
        });
        if changed == Some(true) {
            self.changed();
        } else {
            self.with(|s| s.needs_render = true);
            self.sync_ui();
        }
    }

    fn resize_preview(&self, width: f32, height: f32) {
        let scale = self.context.window().map(|w| w.window().scale_factor()).unwrap_or(1.0);
        self.with(|s| {
            s.preview.set_viewport(width, height, scale);
            s.needs_render = true;
        });
        self.sync_ui();
    }

    // ------------------------------------------------------------ transport

    fn toggle_play(&self) {
        self.with(|s| {
            if s.playing() {
                s.audio.stop();
                s.editor.time = vf_core::time::snap_to_frame(s.editor.time, s.editor.fps());
            } else {
                let end = s.editor.playback_end();
                if end <= 0.0 {
                    return;
                }
                if s.editor.time >= end - s.editor.frame() {
                    s.editor.time = 0.0;
                }
                s.audio.set_project(Arc::new(s.editor.project.clone()));
                let time = s.editor.time;
                s.audio.play(time);
            }
            s.needs_render = true;
        });
        self.sync_ui();
    }

    fn seek(&self, time: f64) {
        self.with(|s| {
            s.editor.seek(time);
            if s.playing() {
                let time = s.editor.time;
                s.audio.play(time);
            }
            s.needs_render = true;
        });
        self.sync_ui();
    }

    fn step(&self, frames: i32) {
        self.with(|s| {
            if s.playing() {
                s.audio.stop();
            }
            s.editor.step_frames(frames);
            s.needs_render = true;
        });
        self.sync_ui();
    }

    fn toggle_fullscreen(&self) {
        let Some(window) = self.context.window() else { return };
        let fullscreen = !window.global::<EditorState>().get_fullscreen();
        window.window().set_fullscreen(fullscreen);
        self.with(|s| {
            s.preview.fullscreen = fullscreen;
            s.needs_render = true;
        });
        window.global::<EditorState>().set_fullscreen(fullscreen);
        self.sync_ui();
    }

    // ------------------------------------------------------------ canvas gestures

    /// Pointer over the canvas without a button: the cursor shows what a press would do.
    fn canvas_hover(&self, x: f64, y: f64) {
        let cursor = self.with(|s| {
            if s.playing() {
                return 0;
            }
            match handle_at(s, x, y) {
                Some((HandlePart::Knob, _)) => 4,
                Some((HandlePart::Corner(index), _)) => {
                    if index % 2 == 0 {
                        2
                    } else {
                        3
                    }
                }
                None => {
                    let project = &s.editor.project;
                    let view = s.preview.view_scale(project).max(1e-6);
                    let layers = s.preview.layers(project, s.editor.time);
                    let hit = canvas::hit_test(&layers, (x / view, y / view), s.editor.primary.as_deref());
                    i32::from(hit.is_some_and(|id| s.editor.editable(id).is_some()))
                }
            }
        });
        if let (Some(cursor), Some(window)) = (cursor, self.context.window()) {
            let state = window.global::<EditorState>();
            if state.get_canvas_cursor() != cursor {
                state.set_canvas_cursor(cursor);
            }
        }
    }

    fn canvas_pointer(&self, phase: PointerPhase, x: f64, y: f64, shift: bool, control: bool) {
        let snap = self.context.settings.get().snap;
        let mut ui = false;
        self.with(|s| {
            let project = &s.editor.project;
            let view = s.preview.view_scale(project).max(1e-6);
            let point = (x / view, y / view);
            let canvas_size = (project.width as f64, project.height as f64);
            match phase {
                PointerPhase::Down => {
                    match handle_at(s, x, y) {
                        Some((HandlePart::Knob, layers)) => {
                            s.gesture = Gesture::begin(Mode::Rotate, point, &layers);
                            return;
                        }
                        Some((HandlePart::Corner(_), layers)) => {
                            s.gesture = Gesture::begin(Mode::Scale, point, &layers);
                            return;
                        }
                        None => {}
                    }
                    let layers = s.preview.layers(project, s.editor.time);
                    let hit = canvas::hit_test(&layers, point, s.editor.primary.as_deref()).map(str::to_string);
                    let top = canvas::hits(&layers, point).first().map(|id| id.to_string());
                    s.click_pick = top.filter(|top| hit.as_ref() != Some(top) && !shift && !control);
                    // Selecting changes the editor, so work on a copy of the document from here on.
                    let project = &project.clone();
                    match hit {
                        None => s.editor.select(None, false),
                        Some(id) if shift || control => s.editor.select(Some(&id), true),
                        Some(id) => {
                            if !s.editor.selection.contains(&id) {
                                s.editor.select(Some(&id), false);
                            }
                            let targets: Vec<_> = s
                                .preview
                                .static_layers(project, s.editor.time, &s.editor.selection)
                                .into_iter()
                                .filter(|(id, _)| s.editor.editable(id).is_some())
                                .collect();
                            s.gesture = Gesture::begin(Mode::Move, point, &targets);
                        }
                    }
                    s.needs_render = true;
                    ui = true;
                }
                PointerPhase::Move => {
                    let Some(gesture) = s.gesture.as_mut() else { return };
                    let first = !gesture.changed;
                    if let Some((patches, guides)) = gesture.update(point, canvas_size, 1.0 / view, snap && !control, shift) {
                        if first {
                            s.editor.begin_change();
                        }
                        s.editor.set_transform(&patches, false);
                        s.guides = guides;
                        s.needs_render = true;
                        ui = true;
                    }
                }
                PointerPhase::Up => {
                    let moved = s.gesture.as_ref().is_some_and(|g| g.changed);
                    s.gesture = None;
                    s.guides = Default::default();
                    if let Some(top) = s.click_pick.take().filter(|_| !moved) {
                        s.editor.select(Some(&top), false);
                        s.needs_render = true;
                    }
                    ui = true;
                }
                PointerPhase::Double => {}
            }
        });
        if ui {
            self.changed();
        }
    }

    // ------------------------------------------------------------ media

    fn import_media(&self, add_to_timeline: bool) {
        let paths = media::pick_files();
        self.import_paths(&paths, add_to_timeline);
    }

    pub fn import_paths(&self, paths: &[PathBuf], add_to_timeline: bool) -> Vec<MediaAsset> {
        if paths.is_empty() {
            return vec![];
        }
        let known = self.with(|s| s.editor.project.assets.clone()).unwrap_or_default();
        let imported = media::import(paths, &known);
        if !imported.failures.is_empty() {
            let title = if imported.failures.len() == 1 {
                "A file could not be imported".to_string()
            } else {
                format!("{} files could not be imported", imported.failures.len())
            };
            self.context.toasts.error(&title, &imported.failures.iter().take(2).cloned().collect::<Vec<_>>().join("\n"));
        }
        let assets = imported.assets.clone();
        self.edit(|e| {
            e.register_assets(assets.clone());
            if add_to_timeline {
                e.add_assets_sequence(&assets, Placement { start: Some(0.0), ..Default::default() });
            }
        });
        if !add_to_timeline && !imported.assets.is_empty() {
            let title =
                if imported.assets.len() == 1 { format!("{} imported", imported.assets[0].name) } else { format!("{} files imported", imported.assets.len()) };
            self.context.toasts.success(&title, "");
        }
        imported.assets
    }

    fn more(&self, id: &str) {
        match id {
            "import" => self.import_media(false),
            "save-file" => self.save_project_file(),
            "shortcuts" => self.show_shortcuts(),
            "settings" => {
                self.flush();
                self.context.go(Page::Settings);
            }
            _ => {}
        }
    }

    fn show_shortcuts(&self) {
        if let Some(window) = self.context.window() {
            window.global::<EditorState>().set_shortcuts_open(true);
        }
    }

    fn save_project_file(&self) {
        let Some(project) = self.with(|s| s.editor.project.clone()) else { return };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save project file")
            .set_file_name(format!("{}.vfproj", project.name))
            .add_filter("ViralFlow project", &["vfproj", "json"])
            .save_file()
        else {
            return;
        };
        match projects::write_file(&path, &project) {
            Ok(()) => self.context.toasts.success("Project file saved", &path.display().to_string()),
            Err(error) => self.context.toasts.error("Project file could not be saved", &error.to_string()),
        }
    }

    // ------------------------------------------------------------ keyboard

    fn key(&self, text: &str, control: bool, shift: bool, alt: bool) -> bool {
        use slint::platform::Key;
        let is = |key: Key| text == slint::SharedString::from(key).as_str();
        let lower = text.to_lowercase();
        if control {
            match lower.as_str() {
                "z" if shift => self.edit(|e| {
                    e.redo();
                }),
                "z" => self.edit(|e| {
                    e.undo();
                }),
                "y" => self.edit(|e| {
                    e.redo();
                }),
                "c" => {
                    self.with(|s| s.editor.copy_selected());
                }
                "x" => self.edit(|e| e.cut_selected()),
                "v" => self.edit(|e| e.paste()),
                "d" => self.edit(|e| e.duplicate_selected()),
                "a" => self.edit(|e| e.select_all()),
                "g" if shift => self.edit(|e| e.ungroup_selected()),
                "g" => self.edit(|e| e.group_selected()),
                "b" => self.edit(|e| {
                    let time = e.time;
                    e.split_at(time)
                }),
                "s" => {
                    self.flush();
                    if self.with(|s| !s.save_failed).unwrap_or(false) {
                        self.context.toasts.success("Project saved", "");
                    }
                    self.sync_ui();
                }
                "i" => self.import_media(false),
                "e" => self.open_export(),
                _ => return false,
            }
            return true;
        }
        if text == " " {
            self.toggle_play();
        } else if text == "?" {
            self.show_shortcuts();
        } else if lower == "s" {
            self.edit(|e| {
                let time = e.time;
                e.split_at(time)
            });
        } else if lower == "q" {
            self.edit(|e| e.trim_to_playhead(Edge::Start));
        } else if lower == "w" {
            self.edit(|e| e.trim_to_playhead(Edge::End));
        } else if lower == "k" {
            if shift {
                self.edit(|e| e.jump_keyframe(-1));
            } else if alt {
                self.edit(|e| e.jump_keyframe(1));
            } else {
                self.edit(|e| e.toggle_keyframe_at_playhead());
            }
        } else if is(Key::Delete) || is(Key::Backspace) {
            self.edit(|e| e.delete_selected(shift));
        } else if is(Key::LeftArrow) || is(Key::RightArrow) {
            let direction = if is(Key::LeftArrow) { -1 } else { 1 };
            if alt {
                self.edit(|e| {
                    let frame = e.frame();
                    e.nudge_selected(direction as f64 * frame)
                });
            } else if shift {
                let time = self.with(|s| s.editor.time).unwrap_or(0.0);
                self.seek(time + direction as f64);
            } else {
                self.step(direction);
            }
        } else if is(Key::Home) {
            self.seek(0.0);
        } else if is(Key::End) {
            let end = self.with(|s| s.editor.playback_end()).unwrap_or(0.0);
            self.seek(end);
        } else if text == "=" || text == "+" || text == "-" || (lower == "z" && shift) {
            let action = if text == "-" {
                "zoom-out"
            } else if lower == "z" {
                "fit"
            } else {
                "zoom-in"
            };
            self.timeline_action(action);
        } else if is(Key::Escape) {
            if self.with(|s| s.drag.as_ref().is_some_and(|d| d.started)).unwrap_or(false) {
                self.cancel_drag();
            } else if self.with(|s| s.timeline.cancel(&mut s.editor)).unwrap_or(false) {
                self.changed();
            } else if self.context.window().is_some_and(|w| w.global::<EditorState>().get_fullscreen()) {
                self.toggle_fullscreen();
            } else {
                self.edit(|e| e.select(None, false));
            }
        } else if lower == "h" && !shift {
            self.edit(|e| e.toggle_item_flag(Flag::Hidden));
        } else {
            return false;
        }
        true
    }

    // ------------------------------------------------------------ UI

    fn sync_ui(&self) {
        let Some(window) = self.context.window() else { return };
        let state = window.global::<EditorState>();
        let settings = self.context.settings.get();
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else { return };
        let editor = &session.editor;
        let project = &editor.project;
        if session.preview.layout(project) {
            session.needs_render = true;
        }
        state.set_name(project.name.clone().into());
        state.set_save_state(if session.save_failed {
            SaveState::Error
        } else if editor.revision() != session.saved_revision {
            SaveState::Dirty
        } else {
            SaveState::Saved
        });
        state.set_canvas_width(session.preview.canvas.0);
        state.set_canvas_height(session.preview.canvas.1);
        state.set_format_label(format!("{} × {} · {} fps", project.width, project.height, project.fps).into());
        state.set_aspect_label(aspect_label(project.width, project.height).into());
        state.set_empty(project.items().next().is_none());
        state.set_playing(session.playing());
        state.set_time(editor.time as f32);
        state.set_end(editor.playback_end() as f32);
        state.set_time_label(format::time(editor.time, Some(project.fps)).into());
        state.set_end_label(format::time(editor.playback_end(), Some(project.fps)).into());
        state.set_can_undo(editor.can_undo());
        state.set_can_redo(editor.can_redo());
        state.set_muted(session.muted);
        state.set_safe_area(settings.safe_area);
        let view = session.preview.view_scale(project);
        state.set_guide_x(session.guides.x.map(|v| (v * view) as f32).unwrap_or(-1.0));
        state.set_guide_y(session.guides.y.map(|v| (v * view) as f32).unwrap_or(-1.0));
        // Selection outlines (hidden while playing, like the old preview).
        let handles: Vec<Handle> = if session.playing() {
            vec![]
        } else {
            session
                .preview
                .static_layers(project, editor.time, &editor.selection)
                .iter()
                .map(|(id, layer)| {
                    let q = layer_quad(layer).map(|(x, y)| ((x * view) as f32, (y * view) as f32));
                    Handle {
                        x0: q[0].0,
                        y0: q[0].1,
                        x1: q[1].0,
                        y1: q[1].1,
                        x2: q[2].0,
                        y2: q[2].1,
                        x3: q[3].0,
                        y3: q[3].1,
                        rotation: layer.rotation as f32,
                        primary: Some(id) == editor.primary.as_ref(),
                        locked: editor.editable(id).is_none(),
                    }
                })
                .collect()
        };
        // Updated in place: replacing the model would drop the grab of a corner being dragged.
        if !session.handles_bound {
            session.handles_bound = true;
            state.set_handles(ModelRc::from(session.handles.clone()));
        }
        crate::models::sync_rows(&session.handles, handles);
        let timeline = window.global::<TimelineState>();
        session.timeline.clamp_scroll(&session.editor);
        session.timeline.sync(&session.editor, &timeline);
        if !session.playing() {
            session.stop_at = None;
        }
        let inspector_state = window.global::<InspectorState>();
        inspector_state.set_can_rewrite(self.context.keys.status().has_key);
        inspector::sync(&session.editor, &inspector_state, &mut session.inspector_key);
        assets::sync(&session.editor, &mut session.timeline.overviews, &session.offline, &window.global::<AssetsState>(), &mut session.assets_key);
    }
}

/// Part of the selection outline under the pointer.
enum HandlePart {
    /// Corner 0..4, clockwise from the top left.
    Corner(usize),
    Knob,
}

/// The corner or rotation knob of the single selected (unlocked) layer at
/// canvas point `x`, `y` (logical pixels), with the layer it belongs to.
fn handle_at(s: &Session, x: f64, y: f64) -> Option<(HandlePart, Vec<(String, vf_core::engine::scene::Layer)>)> {
    let project = &s.editor.project;
    if s.editor.selection.len() != 1 {
        return None;
    }
    let view = s.preview.view_scale(project).max(1e-6);
    let layers: Vec<_> =
        s.preview.static_layers(project, s.editor.time, &s.editor.selection).into_iter().filter(|(id, _)| s.editor.editable(id).is_some()).collect();
    let [(_, layer)] = layers.as_slice() else { return None };
    let quad = layer_quad(layer).map(|(qx, qy)| (qx * view, qy * view));
    let near = |p: (f64, f64), r: f64| (p.0 - x).hypot(p.1 - y) <= r;
    if near(canvas::knob(&quad, layer.rotation, 1.0), 12.0) {
        return Some((HandlePart::Knob, layers));
    }
    let corner = quad.iter().position(|corner| near(*corner, 8.0))?;
    Some((HandlePart::Corner(corner), layers))
}

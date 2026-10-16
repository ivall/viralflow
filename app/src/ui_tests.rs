//! Headless UI tests: the Slint UI runs with the testing backend (no window on
//! screen) and gets simulated pointer input. The backend can be set up once per
//! process, so all scenarios run inside one test, one after another.

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::ElementHandle;
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, Model};

use crate::{AppState, AppWindow, EditorState, Page, TimelineState};

fn editor_window() -> AppWindow {
    let window = AppWindow::new().expect("window");
    window.window().set_size(slint::LogicalSize::new(1440.0, 900.0));
    window.global::<AppState>().set_page(Page::Editor);
    window.global::<EditorState>().set_loading(false);
    // Shown in memory; conditional elements are created when laid out.
    window.show().expect("show");
    window
}

fn element(window: &AppWindow, id: &str) -> ElementHandle {
    ElementHandle::find_by_element_id(window, id).next().unwrap_or_else(|| panic!("no element {id}"))
}

fn labelled(window: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label).next().unwrap_or_else(|| panic!("nothing labelled {label}"))
}

/// Calls made by the UI, in order.
type Log = Rc<RefCell<Vec<String>>>;

fn export_button_reacts_without_dragging_the_window() {
    let window = editor_window();
    let log: Log = Rc::default();
    let (a, b) = (log.clone(), log.clone());
    window.global::<EditorState>().on_export(move || a.borrow_mut().push("export".into()));
    window.global::<AppState>().on_drag_window(move || b.borrow_mut().push("drag".into()));
    element(&window, "TitleBar::export-button").mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), ["export"]);
}

/// Clicks a point given in window coordinates.
fn click_at(window: &AppWindow, x: f32, y: f32, button: PointerEventButton) {
    use slint::platform::WindowEvent;
    let position = slint::LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed { position, button });
    window.window().dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// Centre of an element inside a popup, in window coordinates. The testing
/// backend reports popup elements relative to the popup, which opens at
/// `origin` (in window coordinates).
fn popup_point(entry: &ElementHandle, origin: (f32, f32)) -> (f32, f32) {
    let (p, s) = (entry.absolute_position(), entry.size());
    (origin.0 + p.x + s.width / 2.0, origin.1 + p.y + s.height / 2.0)
}

fn more_menu_reports_the_entry() {
    let window = editor_window();
    let log: Log = Rc::default();
    let l = log.clone();
    window.global::<EditorState>().on_more(move |id| l.borrow_mut().push(id.to_string()));
    let more = element(&window, "TitleBar::more");
    let menu = element(&window, "TitleBar::more-menu");
    more.mock_single_click(PointerEventButton::Left);
    // `more-menu.open(self.x - 200px + self.width, self.height + 4px)` relative to the menu element.
    let origin = (menu.absolute_position().x - 200.0 + more.size().width, menu.absolute_position().y + more.size().height + 4.0);
    let (x, y) = popup_point(&labelled(&window, "Keyboard shortcuts"), origin);
    click_at(&window, x, y, PointerEventButton::Left);
    assert_eq!(*log.borrow(), ["shortcuts"]);
}

fn timeline_context_menu_reports_the_entry() {
    let window = editor_window();
    let log: Log = Rc::default();
    let l = log.clone();
    let state = window.global::<TimelineState>();
    state.set_context_entries(slint::ModelRc::new(slint::VecModel::from(vec![crate::MenuEntry {
        id: "split".into(),
        label: "Split".into(),
        enabled: true,
        ..Default::default()
    }])));
    state.on_context_action(move |id| l.borrow_mut().push(id.to_string()));
    let lanes = element(&window, "Timeline::area-touch");
    lanes.mock_single_click(PointerEventButton::Right);
    labelled(&window, "Split").mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), ["split"]);
}

fn full_screen_shows_only_the_preview() {
    let window = editor_window();
    assert!(ElementHandle::find_by_accessible_label(&window, "Project settings").next().is_some());
    window.global::<EditorState>().set_fullscreen(true);
    let _ = ElementHandle::find_by_element_id(&window, "TitleBar::more").count();
    // Panels, inspector, timeline and the title bar are gone; the transport stays.
    assert!(ElementHandle::find_by_accessible_label(&window, "Project settings").next().is_none(), "the inspector is hidden");
    assert!(ElementHandle::find_by_element_id(&window, "TitleBar::export-button").next().is_none(), "the title bar is hidden");
    assert!(ElementHandle::find_by_element_id(&window, "Timeline::area-touch").next().is_none(), "the timeline is hidden");
    assert!(
        ElementHandle::find_by_accessible_label(&window, "00:00:00").next().is_some()
            || ElementHandle::find_by_element_id(&window, "Player::canvas").next().is_some()
    );
    window.global::<EditorState>().set_fullscreen(false);
    assert!(ElementHandle::find_by_accessible_label(&window, "Project settings").next().is_some(), "the editor comes back");
}

fn asset_tabs_switch_the_panel() {
    let window = editor_window();
    for (label, panel) in
        [("Text", "text"), ("Stickers", "stickers"), ("Effects", "effects"), ("Transitions", "transitions"), ("Captions", "captions"), ("Media", "media")]
    {
        let tab = ElementHandle::find_by_element_id(&window, "PanelTab::tab-touch")
            .find(|candidate| {
                let (p, size) = (candidate.absolute_position(), candidate.size());
                ElementHandle::find_by_accessible_label(&window, label).any(|text| {
                    let (t, ts) = (text.absolute_position(), text.size());
                    let (cx, cy) = (t.x + ts.width / 2.0, t.y + ts.height / 2.0);
                    cx >= p.x && cx <= p.x + size.width && cy >= p.y && cy <= p.y + size.height
                })
            })
            .unwrap_or_else(|| panic!("no tab {label}"));
        tab.mock_single_click(PointerEventButton::Left);
        assert_eq!(window.global::<EditorState>().get_panel(), panel);
    }
}

fn the_title_bar_is_off_while_a_dialog_is_open() {
    let window = editor_window();
    let log: Log = Rc::default();
    let (a, b) = (log.clone(), log.clone());
    window.global::<EditorState>().on_back(move || a.borrow_mut().push("back".into()));
    window.global::<EditorState>().on_export(move || b.borrow_mut().push("export".into()));
    window.global::<crate::ExportState>().set_open(true);
    labelled(&window, "Projects").mock_single_click(PointerEventButton::Left);
    element(&window, "TitleBar::export-button").mock_single_click(PointerEventButton::Left);
    assert!(log.borrow().is_empty(), "title bar reacted under a dialog: {:?}", log.borrow());
    window.global::<crate::ExportState>().set_open(false);
    element(&window, "TitleBar::export-button").mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), ["export"], "and works again once it closes");
}

fn dialogs_close_from_their_buttons() {
    let window = editor_window();
    let log: Log = Rc::default();
    let l = log.clone();
    window.global::<crate::ExportState>().on_close(move || l.borrow_mut().push("export-close".into()));
    window.global::<crate::ExportState>().set_open(true);
    labelled(&window, "Cancel").mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), ["export-close"]);
    window.global::<crate::ExportState>().set_open(false);
    window.global::<EditorState>().set_shortcuts_open(true);
    assert!(ElementHandle::find_by_accessible_label(&window, "Keyboard shortcuts").next().is_some());
}

/// Press, move and release with the left button, like a drag.
fn drag(window: &AppWindow, from: (f32, f32), to: (f32, f32)) {
    use slint::platform::WindowEvent;
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerMoved { position: at(from.0, from.1) });
    window.window().dispatch_event(WindowEvent::PointerPressed { position: at(from.0, from.1), button: PointerEventButton::Left });
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        window.window().dispatch_event(WindowEvent::PointerMoved { position: at(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t) });
    }
    window.window().dispatch_event(WindowEvent::PointerReleased { position: at(to.0, to.1), button: PointerEventButton::Left });
}

fn centre(element: &ElementHandle) -> (f32, f32) {
    let (p, s) = (element.absolute_position(), element.size());
    (p.x + s.width / 2.0, p.y + s.height / 2.0)
}

fn media_tiles_drag_and_add() {
    let window = editor_window();
    let assets = window.global::<crate::AssetsState>();
    let tile = crate::AssetTile { id: "asset-1".into(), name: "clip.mp4".into(), kind: 0, duration: "0:05".into(), ratio: 16.0 / 9.0, ..Default::default() };
    assets.set_assets(slint::ModelRc::new(slint::VecModel::from(vec![tile])));
    assets.set_has_media(true);
    let log: Log = Rc::default();
    let (a, b, c, d) = (log.clone(), log.clone(), log.clone(), log.clone());
    assets.on_press(move |kind, id, _, _| a.borrow_mut().push(format!("press {kind} {id}")));
    assets.on_drag_to(move |_, _| {
        if b.borrow().last().map(String::as_str) != Some("drag") {
            b.borrow_mut().push("drag".into());
        }
    });
    assets.on_release(move |_, _| c.borrow_mut().push("release".into()));
    assets.on_add(move |kind, id| d.borrow_mut().push(format!("add {kind} {id}")));
    let source = element(&window, "MediaTile::touch");
    let start = centre(&source);
    drag(&window, start, (900.0, 700.0));
    assert_eq!(*log.borrow(), ["press asset asset-1", "drag", "release"], "dragging a media tile");
    log.borrow_mut().clear();
    // Two quick clicks (mock time advances 50 ms per click) make a double click.
    source.mock_single_click(PointerEventButton::Left);
    source.mock_single_click(PointerEventButton::Left);
    assert!(log.borrow().iter().any(|entry| entry == "add asset asset-1"), "double click adds the clip: {:?}", log.borrow());
}

/// Clicks the button with this label (a real pointer click at its centre, so a
/// button hidden under something else fails) and returns what Rust was told.
fn press_button(window: &AppWindow, log: &Log, label: &str) -> Vec<String> {
    log.borrow_mut().clear();
    let button = ElementHandle::find_by_accessible_label(window, label)
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button {label:?}"));
    button.mock_single_click(PointerEventButton::Left);
    log.borrow().clone()
}

fn every_editor_button_reaches_rust() {
    let window = editor_window();
    let log: Log = Rc::default();
    let push = |log: &Log| {
        let log = log.clone();
        move |entry: String| log.borrow_mut().push(entry)
    };
    let editor = window.global::<EditorState>();
    let p = push(&log);
    editor.on_go_start(move || p("go-start".into()));
    let p = push(&log);
    editor.on_go_end(move || p("go-end".into()));
    let p = push(&log);
    editor.on_step(move |frames| p(format!("step {frames}")));
    let p = push(&log);
    editor.on_toggle_mute(move || p("mute".into()));
    let p = push(&log);
    editor.on_toggle_fullscreen(move || p("fullscreen".into()));
    let timeline = window.global::<TimelineState>();
    timeline.set_can_undo(true);
    timeline.set_can_redo(true);
    timeline.set_has_primary(true);
    timeline.set_has_selection(true);
    timeline.set_primary_visual(true);
    let p = push(&log);
    timeline.on_action(move |action| p(format!("timeline {action}")));
    let assets = window.global::<crate::AssetsState>();
    let p = push(&log);
    assets.on_import(move || p("import".into()));
    let p = push(&log);
    assets.on_add(move |kind, id| p(format!("add {kind} {id}")));
    let p = push(&log);
    assets.on_action(move |action| p(format!("panel {action}")));

    let expect = |label: &str, wanted: &str| {
        let got = press_button(&window, &log, label);
        assert_eq!(got, [wanted], "button {label:?}");
    };
    // Transport
    expect("Go to start (Home)", "go-start");
    expect("Previous frame (←)", "step -1");
    expect("Next frame (→)", "step 1");
    expect("Go to end (End)", "go-end");
    expect("Mute preview", "mute");
    expect("Full screen", "fullscreen");
    let grid = editor.get_grid();
    press_button(&window, &log, "Grid");
    assert_ne!(editor.get_grid(), grid, "the grid toggles");
    // Timeline toolbar
    for (label, action) in [
        ("Undo (Ctrl+Z)", "undo"),
        ("Redo (Ctrl+Shift+Z)", "redo"),
        ("Split (S)", "split"),
        ("Trim start to playhead (Q)", "trim-start"),
        ("Trim end to playhead (W)", "trim-end"),
        ("Duplicate (Ctrl+D)", "duplicate"),
        ("Delete (Del)", "delete"),
        ("Add or remove keyframe (K)", "keyframe"),
        ("Transitions", "transitions"),
        ("Zoom out (-)", "zoom-out"),
        ("Zoom in (=)", "zoom-in"),
        ("Fit timeline (Shift+Z)", "fit"),
    ] {
        expect(label, &format!("timeline {action}"));
    }
    // Asset panels
    expect("Import media", "import");
    editor.set_panel("text".into());
    expect("Add text", "add text ");
    editor.set_panel("effects".into());
    assets.set_effect_targets(1);
    assets.set_current_effect("punch".into());
    expect("Remove effect", "panel remove-effect");
    editor.set_panel("transitions".into());
    assets.set_has_seam(true);
    assets.set_seam_kind("fade".into());
    assets.set_can_apply_all(true);
    expect("Remove", "panel remove-transition");
    expect("Apply to all cuts", "panel transition-all");
}

fn every_panel_item_can_be_dragged() {
    let window = editor_window();
    let assets = window.global::<crate::AssetsState>();
    let choice = |value: &str| crate::Choice { value: value.into(), label: value.into(), ..Default::default() };
    assets.set_text_presets(slint::ModelRc::new(slint::VecModel::from(vec![crate::PresetCard {
        id: "gold".into(),
        label: "Gold".into(),
        ..Default::default()
    }])));
    assets.set_shapes(slint::ModelRc::new(slint::VecModel::from(vec![crate::ShapeCard {
        kind: "circle".into(),
        label: "Circle".into(),
        ..Default::default()
    }])));
    assets.set_stickers(slint::ModelRc::new(slint::VecModel::from(vec![crate::StickerGroup {
        title: "Reactions".into(),
        emoji: slint::ModelRc::new(slint::VecModel::from(vec![slint::SharedString::from("🔥")])),
    }])));
    assets.set_effects(slint::ModelRc::new(slint::VecModel::from(vec![choice("vhs")])));
    assets.set_transitions(slint::ModelRc::new(slint::VecModel::from(vec![choice("fade")])));
    let log: Log = Rc::default();
    let l = log.clone();
    assets.on_press(move |kind, id, _, _| l.borrow_mut().push(format!("{kind} {id}")));
    for (panel, id, wanted) in [
        ("text", "TextPanel::card-touch", "text gold"),
        ("stickers", "StickersPanel::shape-touch", "shape circle"),
        ("stickers", "StickersPanel::emoji-touch", "sticker 🔥"),
        ("effects", "EffectsPanel::effect-touch", "effect vhs"),
        ("transitions", "TransitionsPanel::preset-touch", "transition fade"),
    ] {
        window.global::<EditorState>().set_panel(panel.into());
        log.borrow_mut().clear();
        let source = element(&window, id);
        let start = centre(&source);
        drag(&window, start, (900.0, 700.0));
        assert_eq!(*log.borrow(), [wanted], "dragging {id}");
    }
}

/// A one-second video made with the export encoder.
fn test_video(dir: &std::path::Path) -> std::path::PathBuf {
    use vf_media::encode::{Container, EncodeSettings, Encoder, VideoCodec};
    let path = dir.join("clip.mp4");
    let settings = EncodeSettings {
        path: path.clone(),
        width: 64,
        height: 36,
        fps: 30,
        codec: VideoCodec::H264,
        container: Container::Mp4,
        quality: 0,
        hardware: false,
        audio: false,
    };
    let mut encoder = Encoder::create(&settings).expect("encoder");
    let frame = vec![200u8; 64 * 36 * 4];
    for _ in 0..30 {
        encoder.push_video(&frame).expect("frame");
    }
    encoder.finish().expect("finish");
    path
}

/// The real editor controller (GPU renderer, project files in `paths`) on an
/// in-memory window, with `project` open.
fn open_editor(paths: crate::paths::AppPaths, project: &str) -> (AppWindow, Rc<crate::editor::EditorController>) {
    let window = AppWindow::new().unwrap();
    window.window().set_size(slint::LogicalSize::new(1440.0, 900.0));
    let gpu = vf_render::Gpu::new().expect("gpu");
    let toasts = crate::toast::Toasts::new(&window);
    let settings = crate::settings::SettingsStore::load(&paths);
    let keys = crate::secrets::KeyStore::in_memory();
    let context = Rc::new(crate::app::Context { window: window.as_weak(), gpu, paths, settings, keys, toasts });
    let editor = crate::editor::EditorController::new(context, std::sync::Arc::new(vf_render::TextEngine::new()));
    window.show().unwrap();
    editor.open(project);
    editor.step_frame();
    (window, editor)
}

fn temp_paths(name: &str) -> (std::path::PathBuf, crate::paths::AppPaths) {
    let dir = std::env::temp_dir().join(format!("vf-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = crate::paths::AppPaths::in_dir(&dir).unwrap();
    (dir, paths)
}

fn key(window: &AppWindow, text: &str) {
    use slint::platform::WindowEvent;
    window.window().dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window.window().dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

fn text_clip_scales_and_shortcuts_edit() {
    use slint::platform::{Key, WindowEvent};
    let (dir, paths) = temp_paths("text");
    let project = vf_core::presets::create_project("Text test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    // "Add text" in the Text panel adds a 3 s clip at the playhead and selects it.
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    editor.step_frame();
    let text = editor.project().unwrap().items().next().cloned().expect("a text clip");
    assert_eq!(text.kind, vf_core::model::ItemKind::Text);

    // Its bottom-right corner on the preview scales it in one continuous drag.
    let handle = window.global::<EditorState>().get_handles().row_data(0).expect("selection outline");
    let canvas = element(&window, "Player::canvas").absolute_position();
    let corner = (canvas.x + handle.x2, canvas.y + handle.y2);
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerMoved { position: at(corner.0, corner.1) });
    window.window().dispatch_event(WindowEvent::PointerPressed { position: at(corner.0, corner.1), button: PointerEventButton::Left });
    let mut scales = Vec::new();
    for step in 1..=8 {
        let d = step as f32 * 6.0;
        window.window().dispatch_event(WindowEvent::PointerMoved { position: at(corner.0 + d, corner.1 + d) });
        editor.step_frame();
        scales.push(editor.project().unwrap().item(&text.id).unwrap().transform.unwrap().scale);
    }
    window.window().dispatch_event(WindowEvent::PointerReleased { position: at(corner.0 + 48.0, corner.1 + 48.0), button: PointerEventButton::Left });
    assert!(scales.windows(2).all(|pair| pair[1] >= pair[0]) && scales[7] > scales[0] && scales[0] > 1.0, "scale grows through the whole drag: {scales:?}");

    // S splits at the playhead, Ctrl+Z undoes it.
    window.global::<EditorState>().invoke_seek(1.0);
    key(&window, "s");
    assert_eq!(editor.project().unwrap().items().count(), 2, "S splits the clip");
    let control = slint::SharedString::from(Key::Control);
    window.window().dispatch_event(WindowEvent::KeyPressed { text: control.clone() });
    key(&window, "z");
    window.window().dispatch_event(WindowEvent::KeyReleased { text: control });
    assert_eq!(editor.project().unwrap().items().count(), 1, "Ctrl+Z undoes the split");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn press_drag_release(window: &AppWindow, editor: &crate::editor::EditorController, from: (f32, f32), to: (f32, f32)) {
    use slint::platform::WindowEvent;
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerMoved { position: at(from.0, from.1) });
    window.window().dispatch_event(WindowEvent::PointerPressed { position: at(from.0, from.1), button: PointerEventButton::Left });
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        window.window().dispatch_event(WindowEvent::PointerMoved { position: at(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t) });
        editor.step_frame();
    }
    window.window().dispatch_event(WindowEvent::PointerReleased { position: at(to.0, to.1), button: PointerEventButton::Left });
    editor.step_frame();
}

fn timeline_clips_move_and_trim_and_the_inspector_edits() {
    let (dir, paths) = temp_paths("timeline");
    let project = vf_core::presets::create_project("Timeline test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    editor.step_frame();
    let id = editor.project().unwrap().items().next().unwrap().id.clone();
    let clip = || window.global::<TimelineState>().get_clips().iter().find(|c| c.id == id.as_str()).expect("clip on the timeline");
    let lanes = element(&window, "Timeline::area-touch").absolute_position();

    // Dragging the middle of the clip moves it later.
    let view = clip();
    let middle = (lanes.x + view.x + view.width / 2.0, lanes.y + view.y + view.height / 2.0);
    press_drag_release(&window, &editor, middle, (middle.0 + 120.0, middle.1));
    let moved = editor.project().unwrap().item(&id).unwrap().clone();
    assert!(moved.start > 0.5, "the clip moved: start {}", moved.start);

    // Dragging its right edge shortens it.
    let view = clip();
    let edge = (lanes.x + view.x + view.width - 2.0, lanes.y + view.y + view.height / 2.0);
    press_drag_release(&window, &editor, edge, (edge.0 - 60.0, edge.1));
    let trimmed = editor.project().unwrap().item(&id).unwrap().clone();
    assert!(trimmed.duration < moved.duration - 0.2, "the clip got shorter: {} -> {}", moved.duration, trimmed.duration);
    assert!((trimmed.start - moved.start).abs() < 1e-6, "trimming the end keeps the start");

    // The inspector's opacity slider (Transform tab for text) changes the selected clip.
    window.global::<crate::InspectorState>().set_tab("transform".into());
    let label = labelled(&window, "Opacity");
    let row_y = label.absolute_position().y + label.size().height / 2.0;
    let slider = ElementHandle::find_by_element_id(&window, "Slider::drag")
        .find(|touch| {
            let (p, size) = (touch.absolute_position(), touch.size());
            row_y >= p.y && row_y <= p.y + size.height
        })
        .expect("the opacity slider");
    let (p, size) = (slider.absolute_position(), slider.size());
    let quarter = (p.x + 7.0 + (size.width - 14.0) * 0.25, p.y + size.height / 2.0);
    press_drag_release(&window, &editor, quarter, quarter);
    let opacity = editor.project().unwrap().item(&id).unwrap().transform.unwrap().opacity;
    assert!((opacity - 0.25).abs() < 0.03, "opacity {opacity}");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Lets mock time pass (timers fire) until `done` or a real-time limit.
fn wait_until(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(seconds) {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
        slint::platform::update_timers_and_animations();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

fn export_dialog_writes_the_video() {
    let (dir, paths) = temp_paths("export");
    let video = test_video(&dir);
    let asset = crate::media::import(std::slice::from_ref(&video), &[]).assets.remove(0);
    let mut project = vf_core::presets::create_project("Export test", 1080, 1920, 30);
    project.export.resolution = 360;
    let mut editor_model = vf_core::edit::Editor::new(project);
    editor_model.register_assets(vec![asset.clone()]);
    editor_model.add_asset(&asset, vf_core::edit::Placement::default());
    let project = editor_model.project.clone();
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    let target = dir.join("out.mp4");
    editor.open_export();
    editor.set_export_path(target.clone());
    let export = window.global::<crate::ExportState>();
    assert!(export.get_open());
    // The dialog's own Export button (the title bar has one with the same label).
    let start = ElementHandle::find_by_accessible_label(&window, "Export")
        .filter(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .max_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y))
        .expect("the dialog's Export button");
    start.mock_single_click(PointerEventButton::Left);
    assert_eq!(export.get_phase(), crate::ExportPhase::Working, "error: {}", export.get_error());
    let finished = wait_until(120, || export.get_phase() != crate::ExportPhase::Working);
    assert!(finished, "the export finishes");
    assert_eq!(export.get_phase(), crate::ExportPhase::Done, "error: {}", export.get_error());
    let info = vf_media::probe::probe(&target).expect("a playable file");
    assert_eq!((info.width, info.height), (Some(360), Some(640)));
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The start page, settings page and editor wired as `app::run` wires them.
fn open_app(paths: crate::paths::AppPaths) -> (AppWindow, Rc<crate::home::Home>, Rc<crate::settings_page::SettingsPage>, Rc<crate::editor::EditorController>) {
    let window = AppWindow::new().unwrap();
    window.window().set_size(slint::LogicalSize::new(1440.0, 900.0));
    let gpu = vf_render::Gpu::new().expect("gpu");
    let toasts = crate::toast::Toasts::new(&window);
    let settings = crate::settings::SettingsStore::load(&paths);
    let keys = crate::secrets::KeyStore::in_memory();
    let context = Rc::new(crate::app::Context { window: window.as_weak(), gpu, paths, settings, keys, toasts });
    let home = crate::home::Home::new(context.clone());
    let settings_page = crate::settings_page::SettingsPage::new(context.clone());
    let editor = crate::editor::EditorController::new(context, std::sync::Arc::new(vf_render::TextEngine::new()));
    let opener = editor.clone();
    *home.on_open.borrow_mut() = Some(Box::new(move |id| opener.open(&id)));
    window.global::<AppState>().set_page(Page::Home);
    window.show().unwrap();
    home.refresh();
    (window, home, settings_page, editor)
}

fn new_project_and_settings() {
    let (dir, paths) = temp_paths("home");
    let settings_file = paths.data.join("settings.json");
    let projects = paths.projects.clone();
    let (window, _home, _settings, editor) = open_app(paths);
    // A format card on the start page creates a project and opens it.
    labelled(&window, "TikTok, Reels, Shorts").mock_single_click(PointerEventButton::Left);
    editor.step_frame();
    assert_eq!(window.global::<AppState>().get_page(), Page::Editor);
    let project = editor.project().expect("a project is open");
    assert_eq!((project.width, project.height), (1080, 1920));
    assert!(projects.join(&project.id).join("project.json").is_file(), "the new project is saved");
    editor.close();

    // Settings: the autosave switch writes settings.json.
    window.global::<AppState>().set_page(Page::Settings);
    let label = labelled(&window, "Save automatically");
    let row = label.absolute_position().y + label.size().height / 2.0;
    let switch = i_slint_backend_testing::ElementQuery::from_root(&window)
        .match_accessible_role(i_slint_backend_testing::AccessibleRole::Switch)
        .find_all()
        .into_iter()
        .find(|s| {
            let (p, size) = (s.absolute_position(), s.size());
            (p.y - 20.0..p.y + size.height + 20.0).contains(&row)
        })
        .expect("the autosave switch");
    let before = switch.accessible_checked();
    switch.mock_single_click(PointerEventButton::Left);
    let saved = std::fs::read_to_string(&settings_file).expect("settings.json written");
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(value["autosave"].as_bool(), before.map(|b| !b), "autosave flipped and saved");

    // The default format list shows each format's name next to its hint.
    let field = ElementHandle::find_by_accessible_label(&window, "9:16")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .or_else(|| ElementHandle::find_by_accessible_label(&window, "9:16").next())
        .expect("the default format field");
    field.mock_single_click(PointerEventButton::Left);
    let option = ElementHandle::find_by_accessible_label(&window, "9:16")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
        .expect("the 9:16 option");
    let name = option.query_descendants().match_accessible_role(i_slint_backend_testing::AccessibleRole::Text).find_all();
    let widths: Vec<f32> = name.iter().map(|t| t.size().width).collect();
    println!("default format option texts: {:?} widths {widths:?}", name.iter().map(|t| t.accessible_label()).collect::<Vec<_>>());
    assert!(name.iter().any(|t| t.accessible_label().as_deref() == Some("9:16") && t.size().width >= 20.0), "the name is not cut off: {widths:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole editor (controller, GPU renderer, project on disk in a temporary
/// folder) without a window: dragging a media tile onto the timeline adds a clip.
fn media_drags_onto_the_timeline_end_to_end() {
    let (dir, paths) = temp_paths("media");
    let video = test_video(&dir);
    let imported = crate::media::import(std::slice::from_ref(&video), &[]);
    let asset = imported.assets.into_iter().next().expect("probed");
    let mut project = vf_core::presets::create_project("Drop test", 1080, 1920, 30);
    project.assets.push(asset.clone());
    crate::projects::save(&paths.projects, &project).unwrap();

    let (window, editor) = open_editor(paths, &project.id);
    assert_eq!(editor.project().unwrap().items().count(), 0);

    let tile = element(&window, "MediaTile::touch");
    let lanes = element(&window, "Timeline::area-touch");
    let target = centre(&lanes);
    // Drag in steps; the drop targets report where the drag is between moves.
    use slint::platform::WindowEvent;
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    let start = centre(&tile);
    window.window().dispatch_event(WindowEvent::PointerPressed { position: at(start.0, start.1), button: PointerEventButton::Left });
    for step in 1..=20 {
        let t = step as f32 / 20.0;
        window.window().dispatch_event(WindowEvent::PointerMoved { position: at(start.0 + (target.0 - start.0) * t, start.1 + (target.1 - start.1) * t) });
        slint::platform::update_timers_and_animations();
        let _ = ElementHandle::find_by_element_id(&window, "Timeline::area-touch").count();
    }
    window.window().dispatch_event(WindowEvent::PointerReleased { position: at(target.0, target.1), button: PointerEventButton::Left });
    editor.step_frame();
    let count =
        |editor: &crate::editor::EditorController| editor.project().unwrap().items().filter(|item| item.asset_id.as_deref() == Some(asset.id.as_str())).count();
    assert_eq!(count(&editor), 1, "the dropped media is on the timeline");

    // Onto the preview: placed at the playhead.
    let preview = centre(&element(&window, "Player::canvas"));
    let start = centre(&element(&window, "MediaTile::touch"));
    window.window().dispatch_event(WindowEvent::PointerPressed { position: at(start.0, start.1), button: PointerEventButton::Left });
    for step in 1..=20 {
        let t = step as f32 / 20.0;
        window.window().dispatch_event(WindowEvent::PointerMoved { position: at(start.0 + (preview.0 - start.0) * t, start.1 + (preview.1 - start.1) * t) });
        slint::platform::update_timers_and_animations();
    }
    window.window().dispatch_event(WindowEvent::PointerReleased { position: at(preview.0, preview.1), button: PointerEventButton::Left });
    editor.step_frame();
    assert_eq!(count(&editor), 2, "the media dropped on the preview is added");

    // The + button on the tile adds it too.
    let plus = element(&window, "MediaTile::add-touch");
    window.window().dispatch_event(WindowEvent::PointerMoved { position: at(start.0, start.1) });
    plus.mock_single_click(PointerEventButton::Left);
    editor.step_frame();
    assert_eq!(count(&editor), 3, "the + button adds the media");

    // Dragging the second track's header above the first reorders them.
    let tracks = |editor: &crate::editor::EditorController| editor.project().unwrap().tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    let before = tracks(&editor);
    assert!(before.len() >= 2, "{before:?}");
    let headers: Vec<_> = ElementHandle::find_by_element_id(&window, "TrackHeader::header-touch").collect();
    let second = headers.iter().find(|h| h.absolute_position().y > headers[0].absolute_position().y + 5.0).expect("a second header");
    // Grab the header by its grip on the left; its right half holds the eye, mute and lock buttons.
    let grip = |h: &ElementHandle| (h.absolute_position().x + 20.0, h.absolute_position().y + h.size().height / 2.0);
    let (from, to) = (grip(second), (grip(&headers[0]).0, headers[0].absolute_position().y + 2.0));
    press_drag_release(&window, &editor, from, to);
    let after = tracks(&editor);
    assert_eq!(after.len(), before.len());
    assert_ne!(after, before, "the tracks were reordered");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A captioned 2-minute project: one video and many caption clips, like an auto-edit.
fn large_project(paths: &crate::paths::AppPaths, video: &std::path::Path, captions: usize) -> vf_core::model::Project {
    let asset = crate::media::import(&[video.to_path_buf()], &[]).assets.remove(0);
    let mut editor = vf_core::edit::Editor::new(vf_core::presets::create_project("Large", 1080, 1920, 30));
    editor.register_assets(vec![asset.clone()]);
    editor.add_asset(&asset, vf_core::edit::Placement::default());
    let cues: Vec<vf_core::model::CaptionCue> = (0..captions)
        .map(|i| vf_core::model::CaptionCue { text: format!("word{i} and more"), start: i as f64 * 0.4, duration: 0.38, highlight_index: None })
        .collect();
    let source = editor.project.items().next().unwrap().id.clone();
    editor
        .apply_captions(&cues, vf_core::edit::CaptionOptions { source_id: source, offset: 0.0, speed: 1.0, style: vf_core::presets::caption_style(), y: 74.0 });
    crate::projects::save(&paths.projects, &editor.project).unwrap();
    editor.project.clone()
}

/// Prints how long editor frames take on a large project (and fails when
/// they are far from interactive).
fn frame_cost_on_a_large_project() {
    let (dir, paths) = temp_paths("perf");
    let video = test_video(&dir);
    let project = large_project(&paths, &video, 300);
    let (window, editor) = open_editor(paths, &project.id);
    let state = window.global::<EditorState>();
    let measure = |label: &str, frames: usize, mut each: Box<dyn FnMut(usize) + '_>| {
        let started = std::time::Instant::now();
        for frame in 0..frames {
            each(frame);
        }
        let per = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        eprintln!("PERF {label}: {per:.2} ms per frame");
        per
    };
    let idle = measure("idle tick", 30, Box::new(|_| editor.step_frame()));
    let seeking = measure(
        "seek + tick",
        30,
        Box::new(|f| {
            state.invoke_seek(f as f32 * 0.37);
            editor.step_frame();
        }),
    );
    let timeline = window.global::<TimelineState>();
    let scrolling = measure(
        "wheel scroll + tick",
        30,
        Box::new(|_| {
            timeline.invoke_scroll(0.0, -40.0, false, true, 300.0);
            editor.step_frame();
        }),
    );
    eprintln!("PERF clips on timeline model: {}", timeline.get_clips().row_count());
    // Dragging a caption clip: every move is an edit (overlap repair, render, UI).
    let lanes = element(&window, "Timeline::area-touch").absolute_position();
    let clip = timeline.get_clips().iter().find(|c| c.label.contains("word3 ")).expect("a caption clip");
    let from = (lanes.x + clip.x + clip.width / 2.0, lanes.y + clip.y + clip.height / 2.0);
    use slint::platform::WindowEvent;
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerMoved { position: at(from.0, from.1) });
    window.window().dispatch_event(WindowEvent::PointerPressed { position: at(from.0, from.1), button: PointerEventButton::Left });
    let dragging = measure(
        "clip drag move + tick",
        30,
        Box::new(|f| {
            window.window().dispatch_event(WindowEvent::PointerMoved { position: at(from.0 + f as f32 * 3.0, from.1) });
            editor.step_frame();
        }),
    );
    window.window().dispatch_event(WindowEvent::PointerReleased { position: at(from.0 + 90.0, from.1), button: PointerEventButton::Left });
    let undo = measure(
        "undo + redo",
        10,
        Box::new(|_| {
            timeline.invoke_action("undo".into());
            timeline.invoke_action("redo".into());
            editor.step_frame();
        }),
    );
    let project = editor.project().unwrap();
    let saving = measure(
        "project save",
        5,
        Box::new(|_| {
            crate::projects::save(&std::env::temp_dir().join("vf-ui-perf-save"), &project).unwrap();
        }),
    );
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("vf-ui-perf-save"));
    assert!(dragging < 50.0 && undo < 100.0 && saving < 100.0, "slow: drag {dragging:.1}, undo {undo:.1}, save {saving:.1} ms");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(idle < 50.0 && seeking < 200.0 && scrolling < 100.0, "frames are too slow: idle {idle:.1}, seek {seeking:.1}, scroll {scrolling:.1} ms");
}

/// Presses `text` with the given modifier keys held.
fn chord(window: &AppWindow, modifiers: &[slint::platform::Key], text: &str) {
    use slint::platform::WindowEvent;
    for m in modifiers {
        window.window().dispatch_event(WindowEvent::KeyPressed { text: slint::SharedString::from(*m) });
    }
    key(window, text);
    for m in modifiers.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased { text: slint::SharedString::from(*m) });
    }
}

fn keyboard_shortcuts_edit_the_project() {
    use slint::platform::Key;
    let (dir, paths) = temp_paths("keys");
    let project = vf_core::presets::create_project("Keys test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    let project = || editor.project().unwrap();
    let count = || project().items().count();
    let time = || window.global::<EditorState>().get_time() as f64;
    let first = project().items().next().unwrap().id.clone();
    let frame = 1.0 / 30.0;

    chord(&window, &[Key::Control], "d");
    assert_eq!(count(), 2, "Ctrl+D duplicates");
    chord(&window, &[Key::Control], "z");
    assert_eq!(count(), 1, "Ctrl+Z");
    chord(&window, &[Key::Control, Key::Shift], "Z");
    assert_eq!(count(), 2, "Ctrl+Shift+Z redoes");
    chord(&window, &[Key::Control], "y");
    chord(&window, &[Key::Control], "z");
    assert_eq!(count(), 1);

    window.global::<EditorState>().invoke_seek(1.0);
    key(&window, &slint::SharedString::from(Key::RightArrow));
    assert!((time() - (1.0 + frame)).abs() < 1e-3, "→ steps a frame: {}", time());
    key(&window, &slint::SharedString::from(Key::LeftArrow));
    assert!((time() - 1.0).abs() < 1e-3, "← steps back: {}", time());
    chord(&window, &[Key::Shift], &slint::SharedString::from(Key::RightArrow));
    assert!((time() - 2.0).abs() < 1e-3, "Shift+→ jumps a second: {}", time());
    key(&window, &slint::SharedString::from(Key::Home));
    assert!(time().abs() < 1e-6, "Home");
    key(&window, &slint::SharedString::from(Key::End));
    assert!((time() - project().items().next().unwrap().end()).abs() < 1e-3, "End: {}", time());

    window.global::<EditorState>().invoke_seek(1.0);
    key(&window, "k");
    assert_eq!(project().item(&first).unwrap().keyframes.len(), 1, "K adds a keyframe");
    window.global::<EditorState>().invoke_seek(2.0);
    key(&window, "k");
    chord(&window, &[Key::Shift], "K");
    assert!((time() - 1.0).abs() < 1e-3, "Shift+K goes to the previous keyframe: {}", time());

    window.global::<EditorState>().invoke_seek(0.5);
    key(&window, "q");
    assert!((project().item(&first).unwrap().start - 0.5).abs() < 1e-3, "Q trims the start to the playhead");
    window.global::<EditorState>().invoke_seek(2.0);
    key(&window, "w");
    assert!((project().item(&first).unwrap().end() - 2.0).abs() < 1e-3, "W trims the end to the playhead");

    key(&window, "h");
    assert!(project().item(&first).unwrap().hidden, "H hides the clip");
    key(&window, "h");
    assert!(!project().item(&first).unwrap().hidden, "and shows it again");

    chord(&window, &[Key::Control], "c");
    window.global::<EditorState>().invoke_seek(5.0);
    chord(&window, &[Key::Control], "v");
    assert_eq!(count(), 2, "Ctrl+C, Ctrl+V pastes at the playhead");
    assert!(project().items().any(|i| (i.start - 5.0).abs() < 1e-3), "pasted at 5 s");
    chord(&window, &[Key::Control], "a");
    let selected = window.global::<TimelineState>().get_clips().iter().filter(|c| c.selected).count();
    assert_eq!(selected, 2, "Ctrl+A selects everything");
    key(&window, &slint::SharedString::from(Key::Escape));
    assert_eq!(window.global::<TimelineState>().get_clips().iter().filter(|c| c.selected).count(), 0, "Esc clears the selection");
    chord(&window, &[Key::Control], "a");
    key(&window, &slint::SharedString::from(Key::Delete));
    assert_eq!(count(), 0, "Delete removes the selection");

    let zoom = window.global::<TimelineState>().get_zoom_log();
    key(&window, "=");
    assert!(window.global::<TimelineState>().get_zoom_log() > zoom, "= zooms in");
    key(&window, "-");
    key(&window, "-");
    assert!(window.global::<TimelineState>().get_zoom_log() < zoom, "- zooms out");
    key(&window, "?");
    assert!(window.global::<EditorState>().get_shortcuts_open(), "? shows the shortcuts");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn clip_context_menu_and_track_buttons() {
    use slint::platform::WindowEvent;
    let (dir, paths) = temp_paths("context");
    let project = vf_core::presets::create_project("Context test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);

    // Right click on the clip opens its menu at the pointer; entries act on the clip.
    let choose = |label: &str| {
        let lanes = element(&window, "Timeline::area-touch");
        let clip = window.global::<TimelineState>().get_clips().iter().next().expect("a clip");
        let p = lanes.absolute_position();
        let point = (p.x + clip.x + clip.width / 2.0, p.y + clip.y + clip.height / 2.0);
        window.window().dispatch_event(WindowEvent::PointerMoved { position: at(point.0, point.1) });
        window.window().dispatch_event(WindowEvent::PointerPressed { position: at(point.0, point.1), button: PointerEventButton::Right });
        window.window().dispatch_event(WindowEvent::PointerReleased { position: at(point.0, point.1), button: PointerEventButton::Right });
        editor.step_frame();
        // The menu keeps inside the window (it moves up near the bottom), so the
        // entry is activated through its accessibility action, not a guessed point.
        let entry = ElementHandle::find_by_accessible_label(&window, label)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
            .unwrap_or_else(|| panic!("no menu entry {label}"));
        entry.invoke_accessible_default_action();
        editor.step_frame();
    };
    window.global::<EditorState>().invoke_seek(1.0);
    choose("Split at playhead");
    assert_eq!(editor.project().unwrap().items().count(), 2, "Split from the context menu");
    choose("Duplicate");
    assert_eq!(editor.project().unwrap().items().count(), 3, "Duplicate from the context menu");
    choose("Delete");
    assert_eq!(editor.project().unwrap().items().count(), 2, "Delete from the context menu");

    // Track buttons in the header.
    let track = || editor.project().unwrap().tracks[0].clone();
    press_button(&window, &log, "Hide track");
    assert!(track().hidden, "eye hides the track");
    press_button(&window, &log, "Show track");
    assert!(!track().hidden);
    press_button(&window, &log, "Lock track");
    assert!(track().locked, "lock locks it");
    press_button(&window, &log, "Unlock track");
    assert!(!track().locked);
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn inspector_controls_edit_the_text() {
    let (dir, paths) = temp_paths("inspector");
    let project = vf_core::presets::create_project("Inspector test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    editor.step_frame();
    let id = editor.project().unwrap().items().next().unwrap().id.clone();
    let style = || editor.project().unwrap().item(&id).unwrap().text_style();

    // Typing in the text box changes the clip's words.
    let area = element(&window, "TextArea::area");
    area.mock_single_click(PointerEventButton::Left);
    for c in "!!".chars() {
        key(&window, &c.to_string());
    }
    editor.step_frame();
    assert!(style().content.ends_with("!!"), "typed text: {:?}", style().content);

    // The weight list.
    // A real click on the field and on the option (the popup must open on screen).
    let field = labelled(&window, "Extra bold");
    let (at, size) = (field.absolute_position(), field.size());
    field.mock_single_click(PointerEventButton::Left);
    let option = ElementHandle::find_by_accessible_label(&window, "Black")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
        .expect("the Black option");
    let (x, y) = popup_point(&option, (at.x, at.y + size.height + 4.0));
    click_at(&window, x, y, PointerEventButton::Left);
    editor.step_frame();
    assert_eq!(style().weight, 900.0, "weight from the list");

    // The rest of the text settings are further down: scroll the inspector.
    {
        use slint::platform::WindowEvent;
        let over = element(&window, "TextArea::area").absolute_position();
        window.window().dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(over.x + 20.0, over.y + 40.0),
            delta_x: 0.0,
            delta_y: -400.0,
        });
    }
    // Folded section: open it, then pick a highlight mode.
    labelled(&window, "Word highlight").mock_single_click(PointerEventButton::Left);
    labelled(&window, "Box").mock_single_click(PointerEventButton::Left);
    editor.step_frame();
    assert_eq!(style().highlight_mode, vf_core::model::HighlightMode::Box, "highlight mode");

    // Animation tab: an in-animation preset.
    window.global::<crate::InspectorState>().set_tab("animation".into());
    editor.step_frame();
    slint::platform::update_timers_and_animations();
    labelled(&window, "Pop").mock_single_click(PointerEventButton::Left);
    editor.step_frame();
    let anim = editor.project().unwrap().item(&id).unwrap().anim_in;
    assert_eq!(anim.map(|a| a.kind), Some(vf_core::model::AnimationInKind::Pop), "in-animation preset");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn font_picker_and_style_presets_apply() {
    let (dir, paths) = temp_paths("fonts");
    let project = vf_core::presets::create_project("Font test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    editor.step_frame();
    let id = editor.project().unwrap().items().next().unwrap().id.clone();
    let style = || editor.project().unwrap().item(&id).unwrap().text_style();

    // Open the list with a real click on the field, then click a family.
    let trigger = element(&window, "FontPicker::trigger");
    let (at, size) = (trigger.absolute_position(), trigger.size());
    println!("font field at {at:?} size {size:?}");
    click_at(&window, at.x + size.width / 2.0, at.y + size.height / 2.0, PointerEventButton::Left);
    editor.step_frame();
    // A family shown without scrolling (the list has the system fonts too).
    let families: Vec<String> = window.global::<crate::InspectorState>().get_fonts().iter().map(|f| f.to_string()).collect();
    let (family, row) = families
        .iter()
        .filter(|f| **f != style().font)
        .find_map(|f| {
            let row =
                ElementHandle::find_by_accessible_label(&window, f).find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))?;
            (row.absolute_position().y > 40.0 && row.absolute_position().y < 300.0).then(|| (f.clone(), row))
        })
        .expect("a family in view");
    let origin = (at.x, at.y + size.height + 4.0);
    let (x, y) = popup_point(&row, origin);
    println!("row local {:?} -> click {x},{y}", row.absolute_position());
    click_at(&window, x, y, PointerEventButton::Left);
    editor.step_frame();
    assert_eq!(style().font, family, "the clicked family is applied");
    println!("picked {family}");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn home_page_project_actions() {
    use slint::platform::Key;
    let (dir, paths) = temp_paths("projects");
    let root = paths.projects.clone();
    for name in ["Alpha clip", "Beta clip"] {
        crate::projects::save(&root, &vf_core::presets::create_project(name, 1080, 1920, 30)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let names = || {
        let mut names: Vec<String> = crate::projects::list(&root).unwrap().into_iter().map(|p| p.name).collect();
        names.sort();
        names
    };
    let (window, _home, _settings, _editor) = open_app(paths);
    let menu = |label: &str| {
        let actions = ElementHandle::find_by_accessible_label(&window, "Project actions")
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
            .expect("a project actions button");
        actions.mock_single_click(PointerEventButton::Left);
        ElementHandle::find_by_accessible_label(&window, label)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
            .unwrap_or_else(|| panic!("no menu entry {label}"))
            .invoke_accessible_default_action();
    };
    menu("Duplicate");
    assert_eq!(names().len(), 3, "duplicated: {:?}", names());

    menu("Rename");
    let field = element(&window, "HomeView::name-field");
    let input = ElementHandle::find_by_element_id(&window, "TextField::input")
        .find(|e| {
            let (p, f) = (e.absolute_position(), field.absolute_position());
            (p.y - f.y).abs() < 40.0
        })
        .expect("the name field");
    input.mock_single_click(PointerEventButton::Left);
    chord(&window, &[Key::Control], "a");
    for c in "Renamed".chars() {
        key(&window, &c.to_string());
    }
    let log: Log = Rc::default();
    let rename = ElementHandle::find_by_accessible_label(&window, "Rename")
        .filter(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .max_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y))
        .expect("the Rename button");
    rename.mock_single_click(PointerEventButton::Left);
    let _ = &log;
    assert!(names().contains(&"Renamed".to_string()), "renamed: {:?}", names());

    menu("Delete");
    press_button(&window, &log, "Delete project");
    assert_eq!(names().len(), 2, "deleted after confirming: {:?}", names());

    // Search filters the cards.
    let search = ElementHandle::find_by_element_id(&window, "TextField::input").next().expect("search");
    search.mock_single_click(PointerEventButton::Left);
    for c in "beta".chars() {
        key(&window, &c.to_string());
    }
    let cards = window.global::<crate::HomeState>().get_projects();
    assert!(cards.iter().all(|c| c.name.to_lowercase().contains("beta")) && cards.row_count() >= 1, "search shows only matches");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A selected clip filling the frame must not swallow clicks on what lies on top of it.
fn clicks_reach_layers_above_a_selected_video() {
    let (dir, paths) = temp_paths("click-through");
    let video = test_video(&dir);
    let project = vf_core::presets::create_project("Click through", 1080, 1920, 30);
    let asset = crate::media::import(std::slice::from_ref(&video), &[]).assets.remove(0);
    let mut model = vf_core::edit::Editor::new(project);
    model.register_assets(vec![asset.clone()]);
    let video_id = model.add_assets_sequence(&[asset], vf_core::edit::Placement { start: Some(0.0), ..Default::default() }).remove(0);
    let text_id = model.add_item(vf_core::presets::text_item("Hello", None), vf_core::edit::Placement { start: Some(0.0), ..Default::default() }).unwrap();
    crate::projects::save(&paths.projects, &model.project).unwrap();
    let (window, editor) = open_editor(paths, &model.project.id);
    editor.step_frame();
    let element = element(&window, "Player::canvas");
    let (c, size) = (element.absolute_position(), element.size());
    let middle = (c.x + size.width / 2.0, c.y + size.height / 2.0);
    let primary = || editor.with_primary();
    let position = |id: &str| editor.project().unwrap().item(id).unwrap().transform.unwrap_or_default();

    // A click on the text over the selected video selects the text.
    editor.select(&video_id);
    editor.step_frame();
    click_at(&window, middle.0, middle.1, PointerEventButton::Left);
    editor.step_frame();
    assert_eq!(primary().as_deref(), Some(text_id.as_str()), "the text on top is selected");

    // Dragging there while the video is selected still moves the video.
    editor.select(&video_id);
    editor.step_frame();
    let (video_before, text_before) = (position(&video_id), position(&text_id));
    press_drag_release(&window, &editor, middle, (middle.0 + 40.0, middle.1 + 30.0));
    assert_ne!((position(&video_id).x, position(&video_id).y), (video_before.x, video_before.y), "the selected video moved");
    assert_eq!((position(&text_id).x, position(&text_id).y), (text_before.x, text_before.y), "the text stayed");
    assert_eq!(primary().as_deref(), Some(video_id.as_str()), "a drag keeps the selection");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn preview_and_timeline_gestures() {
    use slint::platform::WindowEvent;
    let (dir, paths) = temp_paths("gestures");
    let video = test_video(&dir);
    let mut project = vf_core::presets::create_project("Gestures", 1080, 1920, 30);
    let asset = crate::media::import(std::slice::from_ref(&video), &[]).assets.remove(0);
    let mut model = vf_core::edit::Editor::new(project.clone());
    model.register_assets(vec![asset.clone()]);
    model.add_item(vf_core::presets::text_item("Hello", None), vf_core::edit::Placement::default());
    project = model.project.clone();
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    let id = editor.project().unwrap().items().next().unwrap().id.clone();
    let transform = || editor.project().unwrap().item(&id).unwrap().transform.unwrap_or_default();
    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
    // Geometry is read after the preview has laid itself out.
    editor.step_frame();
    let canvas = || {
        let element = element(&window, "Player::canvas");
        (element.absolute_position(), element.size())
    };
    let outline_centre = || {
        let (c, _) = canvas();
        let h = window.global::<EditorState>().get_handles().row_data(0).expect("an outline");
        (c.x + (h.x0 + h.x2) / 2.0, c.y + (h.y0 + h.y2) / 2.0)
    };

    // Select by clicking the text on the preview, then drag it: it moves...
    let (c, size) = canvas();
    let middle = (c.x + size.width / 2.0, c.y + size.height / 2.0);
    click_at(&window, middle.0, middle.1, PointerEventButton::Left);
    editor.step_frame();
    assert!(window.global::<TimelineState>().get_has_primary(), "clicking the text selects it");
    press_drag_release(&window, &editor, middle, (middle.0 + 40.0, middle.1 + 25.0));
    let moved = transform();
    assert!(moved.x > 52.0 && moved.y > 51.0, "moved to {:?}", (moved.x, moved.y));
    // ...and dragged back near the middle it snaps to the centre lines.
    let grab = outline_centre();
    press_drag_release(&window, &editor, grab, (grab.0 - 40.0 + 1.5, grab.1 - 25.0 - 1.0));
    let snapped = transform();
    assert!((snapped.x - 50.0).abs() < 1e-6 && (snapped.y - 50.0).abs() < 1e-6, "snapped to the centre: {:?}", (snapped.x, snapped.y));

    // The knob above the outline rotates it.
    let (c, _) = canvas();
    let handle = window.global::<EditorState>().get_handles().row_data(0).unwrap();
    let knob = (c.x + (handle.x0 + handle.x1) / 2.0, c.y + (handle.y0 + handle.y1) / 2.0 - 22.0);
    press_drag_release(&window, &editor, knob, (knob.0 + 60.0, knob.1 + 30.0));
    assert!(transform().rotation.abs() > 10.0, "rotated by {}", transform().rotation);

    // Clicking the ruler moves the playhead there.
    let ruler = element(&window, "Timeline::ruler-touch");
    let r = ruler.absolute_position();
    click_at(&window, r.x + 200.0, r.y + 10.0, PointerEventButton::Left);
    editor.step_frame();
    assert!(window.global::<EditorState>().get_time() > 0.5, "the ruler seeks: {}", window.global::<EditorState>().get_time());

    // A marquee from empty space selects the clips it touches.
    window.window().dispatch_event(WindowEvent::KeyPressed { text: slint::SharedString::from(slint::platform::Key::Escape) });
    window.window().dispatch_event(WindowEvent::KeyReleased { text: slint::SharedString::from(slint::platform::Key::Escape) });
    let lanes = element(&window, "Timeline::area-touch");
    let l = lanes.absolute_position();
    let clip = window.global::<TimelineState>().get_clips().iter().next().unwrap();
    let start = (l.x + clip.x + clip.width + 40.0, l.y + lanes.size().height - 10.0);
    press_drag_release(&window, &editor, start, (l.x + clip.x + 4.0, l.y + clip.y + 4.0));
    assert!(window.global::<TimelineState>().get_clips().iter().any(|c| c.selected), "the marquee selects the clip");

    // Export settings: WebM switches the codec to VP9 and the file to .webm.
    editor.open_export();
    let export = window.global::<crate::ExportState>();
    labelled(&window, "WebM").mock_single_click(PointerEventButton::Left);
    assert_eq!(export.get_format(), "webm");
    assert_eq!(export.get_codec(), "vp9");
    assert!(export.get_path().ends_with(".webm"), "path {}", export.get_path());
    let _ = at;
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Playback, muted first so nothing is heard while the test runs.
fn playback_runs_and_stops_at_the_end() {
    let (dir, paths) = temp_paths("playback");
    let mut model = vf_core::edit::Editor::new(vf_core::presets::create_project("Playback", 1080, 1920, 30));
    let mut short = vf_core::presets::text_item("Short", None);
    short.duration = 1.0;
    model.add_item(short, vf_core::edit::Placement::default());
    crate::projects::save(&paths.projects, &model.project).unwrap();
    let (window, editor) = open_editor(paths, &model.project.id);
    let state = window.global::<EditorState>();
    state.invoke_toggle_mute();
    assert!(state.get_muted(), "muted before playing");
    state.invoke_toggle_play();
    assert!(state.get_playing(), "playing");
    let started = std::time::Instant::now();
    let mut times = Vec::new();
    while state.get_playing() && started.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(50));
        editor.step_frame();
        times.push(state.get_time());
    }
    assert!(!state.get_playing(), "playback stops at the end of the timeline");
    assert!(times.windows(2).all(|w| w[1] >= w[0]), "time only moves forward: {times:?}");
    assert!((state.get_time() - 1.0).abs() < 0.05, "stopped at the end: {}", state.get_time());
    // Play again from the end starts over.
    state.invoke_toggle_play();
    editor.step_frame();
    assert!(state.get_time() < 0.5, "starts again from the beginning: {}", state.get_time());
    state.invoke_toggle_play();
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Captions from the panel with the local Whisper model (skipped when the
/// model or the speech recording used by the vf-ai tests is not on this machine).
fn captions_panel_generates_captions() {
    let model = dirs::data_dir().map(|d| d.join("io.github.ivall.viralflow/models/ggml-tiny.bin")).filter(|p| p.is_file());
    let speech = std::env::var("VF_SPEECH").ok().map(std::path::PathBuf::from).filter(|p| p.is_file());
    let (Some(model), Some(speech)) = (model, speech) else {
        eprintln!("captions scenario skipped: set VF_SPEECH and install the tiny model");
        return;
    };
    let (dir, paths) = temp_paths("captions");
    // A copy, so the test never touches the user's models folder.
    std::fs::copy(&model, paths.models.join("ggml-tiny.bin")).unwrap();
    std::fs::write(paths.data.join("settings.json"), r#"{"whisperModel":"tiny","transcriptionProvider":"local","captionLanguage":"en"}"#).unwrap();
    let asset = crate::media::import(std::slice::from_ref(&speech), &[]).assets.remove(0);
    let mut model_editor = vf_core::edit::Editor::new(vf_core::presets::create_project("Captions", 1080, 1920, 30));
    model_editor.register_assets(vec![asset.clone()]);
    model_editor.add_asset(&asset, vf_core::edit::Placement::default());
    crate::projects::save(&paths.projects, &model_editor.project).unwrap();
    let (window, editor) = open_editor(paths, &model_editor.project.id);
    // Opened from the audio inspector's "Auto captions" button, as Rust does it.
    window.global::<crate::InspectorState>().invoke_action("captions".into());
    let captions = window.global::<crate::CaptionsState>();
    assert_eq!(window.global::<EditorState>().get_panel(), "captions");
    assert_eq!(captions.get_sources().row_count(), 1, "the panel is filled when opened from code");
    assert!(captions.get_model_installed(), "the copied model is found");
    assert_ne!(captions.get_source(), "", "the speech clip is offered as the source");
    let log: Log = Rc::default();
    press_button(&window, &log, "Generate captions");
    assert!(captions.get_busy(), "transcribing");
    let done = wait_until(120, || !captions.get_busy());
    assert!(done, "transcription finishes");
    let texts: Vec<String> = editor.project().unwrap().items().filter(|i| i.kind == vf_core::model::ItemKind::Text).map(|i| i.text_style().content).collect();
    assert!(texts.len() >= 3, "captions were added: {texts:?}");
    assert!(texts.join(" ").to_lowercase().contains("captions"), "they say what was spoken: {texts:?}");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn undo_groups_continuous_edits() {
    let (dir, paths) = temp_paths("undo");
    let project = vf_core::presets::create_project("Undo test", 1080, 1920, 30);
    crate::projects::save(&paths.projects, &project).unwrap();
    let (window, editor) = open_editor(paths, &project.id);
    window.global::<EditorState>().set_panel("text".into());
    let log: Log = Rc::default();
    press_button(&window, &log, "Add text");
    let id = editor.project().unwrap().items().next().unwrap().id.clone();
    let opacity = || editor.project().unwrap().item(&id).unwrap().transform.unwrap().opacity;
    window.global::<crate::InspectorState>().set_tab("transform".into());
    editor.step_frame();
    slint::platform::update_timers_and_animations();
    let label = labelled(&window, "Opacity");
    let row_y = label.absolute_position().y + label.size().height / 2.0;
    let slider = ElementHandle::find_by_element_id(&window, "Slider::drag")
        .find(|t| row_y >= t.absolute_position().y && row_y <= t.absolute_position().y + t.size().height)
        .unwrap();
    let (p, size) = (slider.absolute_position(), slider.size());
    let x = |f: f32| p.x + 7.0 + (size.width - 14.0) * f;
    press_drag_release(&window, &editor, (x(0.9), p.y + size.height / 2.0), (x(0.3), p.y + size.height / 2.0));
    assert!((opacity() - 0.3).abs() < 0.03, "dragged to 30%: {}", opacity());
    window.global::<TimelineState>().invoke_action("undo".into());
    assert!((opacity() - 1.0).abs() < 1e-6, "one undo restores the value from before the whole drag: {}", opacity());

    // Typing a word is one undo step too.
    window.global::<crate::InspectorState>().set_tab("main".into());
    editor.step_frame();
    slint::platform::update_timers_and_animations();
    let before = editor.project().unwrap().item(&id).unwrap().text_style().content;
    element(&window, "TextArea::area").mock_single_click(PointerEventButton::Left);
    for c in " word".chars() {
        key(&window, &c.to_string());
    }
    assert_eq!(editor.project().unwrap().item(&id).unwrap().text_style().content, format!("{before} word"));
    window.global::<TimelineState>().invoke_action("undo".into());
    assert_eq!(editor.project().unwrap().item(&id).unwrap().text_style().content, before, "one undo removes the typed word");
    editor.close();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_scenarios() {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    export_button_reacts_without_dragging_the_window();
    more_menu_reports_the_entry();
    timeline_context_menu_reports_the_entry();
    full_screen_shows_only_the_preview();
    asset_tabs_switch_the_panel();
    dialogs_close_from_their_buttons();
    the_title_bar_is_off_while_a_dialog_is_open();
    media_tiles_drag_and_add();
    every_editor_button_reaches_rust();
    every_panel_item_can_be_dragged();
    media_drags_onto_the_timeline_end_to_end();
    text_clip_scales_and_shortcuts_edit();
    timeline_clips_move_and_trim_and_the_inspector_edits();
    new_project_and_settings();
    export_dialog_writes_the_video();
    frame_cost_on_a_large_project();
    keyboard_shortcuts_edit_the_project();
    clip_context_menu_and_track_buttons();
    inspector_controls_edit_the_text();
    font_picker_and_style_presets_apply();
    home_page_project_actions();
    preview_and_timeline_gestures();
    clicks_reach_layers_above_a_selected_video();
    playback_runs_and_stops_at_the_end();
    undo_groups_continuous_edits();
    captions_panel_generates_captions();
}

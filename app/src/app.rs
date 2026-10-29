//! Application setup: the GPU, the window and the controllers of each page.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use vf_core::Result;
use vf_render::Gpu;

use crate::editor::EditorController;
use crate::home::Home;
use crate::paths::AppPaths;
use crate::settings::SettingsStore;
use crate::toast::Toasts;
use crate::{AppState, AppWindow, Page};

/// Shared by every controller of the window.
/// A callback another part of the app can install.
pub type Listener<T> = RefCell<Option<Box<dyn Fn(T)>>>;

pub struct Context {
    pub window: slint::Weak<AppWindow>,
    pub gpu: Gpu,
    pub paths: AppPaths,
    pub settings: SettingsStore,
    /// The OpenRouter API key (system credential store).
    pub keys: crate::secrets::KeyStore,
    pub toasts: Toasts,
    /// The MCP server AI apps connect to.
    pub mcp: crate::agent::mcp::Service,
}

impl Context {
    pub fn window(&self) -> Option<AppWindow> {
        self.window.upgrade()
    }

    pub fn go(&self, page: Page) {
        if let Some(window) = self.window() {
            window.global::<AppState>().set_page(page);
        }
    }
}

pub fn run() -> Result<()> {
    let paths = AppPaths::resolve()?;
    let gpu = Gpu::new()?;

    // Slint draws with our device, so frames rendered by the engine are shown as textures.
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Manual {
            instance: gpu.instance.clone(),
            adapter: gpu.adapter.clone(),
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
        })
        .select()
        .map_err(vf_core::Error::from_display)?;

    let window = AppWindow::new().map_err(vf_core::Error::from_display)?;
    window.global::<crate::Platform>().set_mac(cfg!(target_os = "macos"));
    crate::window::install(&window);
    let toasts = Toasts::new(&window);
    let settings = SettingsStore::load(&paths);
    let keys = crate::secrets::KeyStore::new(&paths.data);
    crate::media::install_builtin_sounds(&paths.builtin);
    let mcp = crate::agent::mcp::Service::new(&paths.data);
    let context = Rc::new(Context { window: window.as_weak(), gpu, paths, settings, keys, toasts, mcp });

    vf_media::init();
    let text = std::sync::Arc::new(vf_render::TextEngine::new());
    let home = Home::new(context.clone());
    let _settings_page = crate::settings_page::SettingsPage::new(context.clone());
    let editor = EditorController::new(context.clone(), text);
    {
        let editor = editor.clone();
        *home.on_open.borrow_mut() = Some(Box::new(move |id| editor.open(&id)));
    }
    {
        let home = Rc::downgrade(&home);
        *editor.on_closed.borrow_mut() = Some(Box::new(move || {
            if let Some(home) = home.upgrade() {
                home.refresh();
            }
        }));
    }
    // The AI editor: the chat in the editor and the tools AI apps reach over MCP.
    let _chat = crate::agent::chat::Chat::new(context.clone(), editor.clone());
    crate::agent::bridge::install(crate::agent::host::Host::new(context.clone(), editor.clone(), home.clone()));
    if context.settings.get().mcp_enabled {
        if let Err(error) = context.mcp.start() {
            log::warn!("MCP server: {error}");
        }
    }
    home.refresh();
    // Development aids: open a page or the auto-edit dialog straight away.
    if std::env::var("VF_PAGE").as_deref() == Ok("settings") {
        context.go(Page::Settings);
    }
    if std::env::var("VF_DIALOG").as_deref() == Ok("autoedit") {
        window.global::<crate::HomeState>().invoke_auto_edit();
    }
    // Development aid: open a project straight away.
    if let Ok(id) = std::env::var("VF_OPEN") {
        editor.open(&id);
        match std::env::var("VF_DIALOG").as_deref() {
            Ok("export") => editor.open_export(),
            Ok("shortcuts") => window.global::<crate::EditorState>().set_shortcuts_open(true),
            _ => {}
        }
        if let Ok(path) = std::env::var("VF_EXPORT") {
            editor.dev_export(path.into());
        }
        if let Ok(panel) = std::env::var("VF_PANEL") {
            window.global::<crate::EditorState>().set_panel(panel.into());
        }
        if let Some(index) = std::env::var("VF_SELECT").ok().and_then(|v| v.parse().ok()) {
            editor.dev_select(index, std::env::var("VF_TAB").ok());
        }
    }

    let closing = editor.clone();
    window.window().on_close_requested(move || {
        // The open project is saved (with its thumbnail) before the app exits.
        closing.close();
        slint::quit_event_loop().ok();
        slint::CloseRequestResponse::HideWindow
    });

    crate::window::snapshot_if_requested(&window);
    let result = window.run().map_err(vf_core::Error::from_display);
    context.mcp.stop();
    crate::agent::bridge::uninstall();
    result?;
    drop(home);
    drop(editor);
    Ok(())
}

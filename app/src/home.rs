//! Start page: new project formats and the project library.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Image, ModelRc, VecModel};
use vf_core::presets::{aspect_label, create_project, ASPECT_PRESETS};

use crate::app::Context;
use crate::projects::{self, ProjectSummary};
use crate::{format, AspectCard, HomeState, Page, ProjectCard};

pub struct Home {
    context: Rc<Context>,
    all: RefCell<Vec<ProjectSummary>>,
    cards: Rc<VecModel<ProjectCard>>,
    /// Decoded thumbnails, so searching does not read every file again.
    thumbnails: RefCell<std::collections::HashMap<std::path::PathBuf, Image>>,
    /// Called when a project should open in the editor.
    pub on_open: crate::app::Listener<String>,
}

impl Home {
    pub fn new(context: Rc<Context>) -> Rc<Self> {
        let home = Rc::new(Home {
            context: context.clone(),
            all: RefCell::default(),
            cards: Rc::new(VecModel::default()),
            thumbnails: RefCell::default(),
            on_open: RefCell::default(),
        });
        let Some(window) = context.window() else { return home };
        let state = window.global::<HomeState>();
        state.set_projects(ModelRc::from(home.cards.clone()));
        let aspects: Vec<AspectCard> =
            ASPECT_PRESETS.iter().map(|p| AspectCard { width: p.width as i32, height: p.height as i32, label: p.label.into(), hint: p.hint.into() }).collect();
        state.set_aspects(ModelRc::new(VecModel::from(aspects)));

        let weak = Rc::downgrade(&home);
        state.on_create(move |width, height| {
            if let Some(home) = weak.upgrade() {
                home.create(width as u32, height as u32);
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_open(move |id| {
            if let Some(home) = weak.upgrade() {
                home.open(&id);
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_rename(move |id, name| {
            if let Some(home) = weak.upgrade() {
                home.rename(&id, &name);
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_duplicate(move |id| {
            if let Some(home) = weak.upgrade() {
                home.duplicate(&id);
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_remove(move |id| {
            if let Some(home) = weak.upgrade() {
                home.remove(&id);
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_export_file(move |id| {
            if let Some(home) = weak.upgrade() {
                home.export_file(&id);
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_import_file(move || {
            if let Some(home) = weak.upgrade() {
                home.import_file();
            }
        });
        let weak = Rc::downgrade(&home);
        state.on_query_changed(move |query| {
            if let Some(home) = weak.upgrade() {
                if let Some(window) = home.context.window() {
                    window.global::<HomeState>().set_query(query);
                }
                home.show();
            }
        });
        let ctx = context.clone();
        state.on_open_settings(move || ctx.go(Page::Settings));
        let auto_edit = crate::autoedit::AutoEdit::new(context.clone());
        {
            let weak = Rc::downgrade(&home);
            *auto_edit.on_finished.borrow_mut() = Some(Box::new(move |open| {
                let Some(home) = weak.upgrade() else { return };
                home.refresh();
                if let Some(id) = open {
                    home.open(&id);
                }
            }));
        }
        state.on_auto_edit(move || auto_edit.open());
        home
    }

    /// Reloads the project list from disk.
    pub fn refresh(&self) {
        match projects::list(&self.context.paths.projects) {
            Ok(list) => {
                // Thumbnails change when a project is saved: read them again on refresh.
                self.thumbnails.borrow_mut().clear();
                *self.all.borrow_mut() = list;
            }
            Err(error) => self.context.toasts.error("Projects could not be listed", &error.to_string()),
        }
        self.show();
    }

    fn show(&self) {
        let query = self.context.window().map(|w| w.global::<HomeState>().get_query().to_lowercase()).unwrap_or_default();
        let cards: Vec<ProjectCard> = self
            .all
            .borrow()
            .iter()
            .filter(|p| query.trim().is_empty() || p.name.to_lowercase().contains(query.trim()))
            .map(|p| {
                let thumbnail = p.thumbnail.as_ref().and_then(|path| {
                    let mut cache = self.thumbnails.borrow_mut();
                    if let Some(image) = cache.get(path) {
                        return Some(image.clone());
                    }
                    let image = Image::load_from_path(path).ok()?;
                    cache.insert(path.clone(), image.clone());
                    Some(image)
                });
                ProjectCard {
                    id: p.id.clone().into(),
                    name: p.name.clone().into(),
                    edited: format::edited(&p.updated_at).into(),
                    duration: format::duration(p.duration).into(),
                    aspect: aspect_label(p.width, p.height).into(),
                    has_thumbnail: thumbnail.is_some(),
                    thumbnail: thumbnail.unwrap_or_default(),
                    width: p.width as i32,
                    height: p.height as i32,
                }
            })
            .collect();
        self.cards.set_vec(cards);
    }

    fn create(&self, width: u32, height: u32) {
        let fps = self.context.settings.get().default_fps;
        let project = create_project("", width, height, if vf_core::model::FPS_CHOICES.contains(&fps) { fps } else { 30 });
        match projects::save(&self.context.paths.projects, &project) {
            Ok(()) => self.open(&project.id),
            Err(error) => self.context.toasts.error("Project could not be created", &error.to_string()),
        }
    }

    fn open(&self, id: &str) {
        if let Some(open) = self.on_open.borrow().as_ref() {
            open(id.to_string());
        } else {
            self.context.toasts.show("The editor arrives in the next step of the migration.");
        }
    }

    fn rename(&self, id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let result = projects::load(&self.context.paths.projects, id).and_then(|mut project| {
            project.name = name.into();
            project.updated_at = vf_core::presets::now_iso();
            projects::save(&self.context.paths.projects, &project)
        });
        if let Err(error) = result {
            self.context.toasts.error("Project could not be renamed", &error.to_string());
        }
        self.refresh();
    }

    fn duplicate(&self, id: &str) {
        let name = self.all.borrow().iter().find(|p| p.id == id).map(|p| format!("{} copy", p.name)).unwrap_or_default();
        if let Err(error) = projects::duplicate(&self.context.paths.projects, id, &vf_core::engine::math::uid("project"), &name) {
            self.context.toasts.error("Project could not be duplicated", &error.to_string());
        }
        self.refresh();
    }

    fn remove(&self, id: &str) {
        match projects::delete(&self.context.paths.projects, id) {
            Ok(()) => self.context.toasts.show("Project deleted"),
            Err(error) => self.context.toasts.error("Project could not be deleted", &error.to_string()),
        }
        self.refresh();
    }

    fn export_file(&self, id: &str) {
        let project = match projects::load(&self.context.paths.projects, id) {
            Ok(project) => project,
            Err(error) => return self.context.toasts.error("Project could not be read", &error.to_string()),
        };
        let file_name = format!("{}.viralflow.json", sanitize_file_name(&project.name));
        let Some(path) = rfd::FileDialog::new().set_title("Save project file").set_file_name(file_name).add_filter("ViralFlow project", &["json"]).save_file()
        else {
            return;
        };
        match projects::write_file(&path, &project) {
            Ok(()) => self.context.toasts.success("Project file saved", &path.display().to_string()),
            Err(error) => self.context.toasts.error("Project file could not be saved", &error.to_string()),
        }
    }

    fn import_file(&self) {
        let Some(path) = rfd::FileDialog::new().set_title("Open project file").add_filter("ViralFlow project", &["json"]).pick_file() else { return };
        let result = projects::read_file(&path).and_then(|mut project| {
            project.id = vf_core::engine::math::uid("project");
            project.updated_at = vf_core::presets::now_iso();
            projects::save(&self.context.paths.projects, &project).map(|()| project)
        });
        match result {
            Ok(project) => {
                self.refresh();
                self.open(&project.id);
            }
            Err(error) => self.context.toasts.error("Project file could not be opened", &error.to_string()),
        }
    }
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "project".into()
    } else {
        trimmed.into()
    }
}

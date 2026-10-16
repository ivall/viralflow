//! Short notifications in the bottom-right corner.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{AppState, AppWindow, Toast, ToastTone};

type Action = Box<dyn Fn()>;

#[derive(Clone)]
pub struct Toasts {
    model: Rc<VecModel<Toast>>,
    actions: Rc<RefCell<Vec<(i32, Action)>>>,
    next: Rc<RefCell<i32>>,
}

impl Toasts {
    pub fn new(window: &AppWindow) -> Self {
        let model = Rc::new(VecModel::default());
        window.global::<AppState>().set_toasts(ModelRc::from(model.clone()));
        let toasts = Toasts { model, actions: Rc::default(), next: Rc::new(RefCell::new(1)) };
        let dismiss = toasts.clone();
        window.global::<AppState>().on_dismiss_toast(move |id| dismiss.dismiss(id));
        let act = toasts.clone();
        window.global::<AppState>().on_toast_action(move |id| {
            let action = act.actions.borrow().iter().position(|(key, _)| *key == id).map(|index| act.actions.borrow_mut().remove(index).1);
            if let Some(action) = action {
                action();
            }
            act.dismiss(id);
        });
        toasts
    }

    fn push(&self, tone: ToastTone, title: &str, detail: &str, action: Option<(&str, Action)>) {
        let id = {
            let mut next = self.next.borrow_mut();
            *next += 1;
            *next
        };
        let action_label = action.as_ref().map(|(label, _)| label.to_string()).unwrap_or_default();
        if let Some((_, run)) = action {
            self.actions.borrow_mut().push((id, run));
        }
        // At most four at a time; the oldest goes first.
        while self.model.row_count() >= 4 {
            let old = self.model.row_data(0).map(|t| t.id).unwrap_or_default();
            self.dismiss(old);
        }
        self.model.push(Toast { id, title: title.into(), detail: detail.into(), tone, action: action_label.into() });
        let lifetime = if tone == ToastTone::Error { 8 } else { 4 };
        let this = self.clone();
        slint::Timer::single_shot(Duration::from_secs(lifetime), move || this.dismiss(id));
    }

    pub fn show(&self, title: &str) {
        self.push(ToastTone::Info, title, "", None);
    }

    pub fn info(&self, title: &str, detail: &str) {
        self.push(ToastTone::Info, title, detail, None);
    }

    pub fn success(&self, title: &str, detail: &str) {
        self.push(ToastTone::Success, title, detail, None);
    }

    pub fn error(&self, title: &str, detail: &str) {
        self.push(ToastTone::Error, title, detail, None);
    }

    pub fn with_action(&self, tone: ToastTone, title: &str, detail: &str, label: &str, run: impl Fn() + 'static) {
        self.push(tone, title, detail, Some((label, Box::new(run))));
    }

    pub fn dismiss(&self, id: i32) {
        if let Some(index) = self.model.iter().position(|toast| toast.id == id) {
            self.model.remove(index);
        }
        self.actions.borrow_mut().retain(|(key, _)| *key != id);
    }
}

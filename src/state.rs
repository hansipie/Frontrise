//! Shared, observable application state.

use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Store;

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
pub struct State {
    pub store: RefCell<Store>,
    ui_listeners: RefCell<Vec<Listener>>,
    engine_hook: RefCell<Option<Listener>>,
}

impl State {
    pub fn load() -> Rc<Self> {
        Rc::new(Self { store: RefCell::new(Store::load()), ..Default::default() })
    }

    /// Changes the store, saves it, updates the wallpapers and rebuilds the UI.
    pub fn modify(&self, f: impl FnOnce(&mut Store)) {
        self.apply(f, true);
    }

    /// Like `modify`, but does not rebuild the UI. For edits made by a widget
    /// that already shows the new value (switches, spin buttons, combos).
    pub fn modify_quiet(&self, f: impl FnOnce(&mut Store)) {
        self.apply(f, false);
    }

    fn apply(&self, f: impl FnOnce(&mut Store), notify_ui: bool) {
        {
            let mut store = self.store.borrow_mut();
            f(&mut store);
            store.save();
        }
        let hook = self.engine_hook.borrow().clone();
        if let Some(hook) = hook {
            hook();
        }
        if notify_ui {
            self.notify_ui();
        }
    }

    pub fn notify_ui(&self) {
        let listeners: Vec<Listener> = self.ui_listeners.borrow().clone();
        for listener in listeners {
            listener();
        }
    }

    pub fn connect_ui(&self, f: impl Fn() + 'static) {
        self.ui_listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn set_engine_hook(&self, f: impl Fn() + 'static) {
        *self.engine_hook.borrow_mut() = Some(Rc::new(f));
    }
}

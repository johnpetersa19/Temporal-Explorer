/* history_controls.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! HistoryControls — prev/next commit navigation widget.
//!
//! Emits:
//!   - `"navigate-back"`    (no args) — user pressed the back button
//!   - `"navigate-forward"` (no args) — user pressed the forward button

use adw::subclass::prelude::*;
use gtk::{glib, glib::subclass::Signal};
use gtk::prelude::{ButtonExt, ObjectExt, WidgetExt};
use std::sync::OnceLock;

// ── Private implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/history-controls.ui")]
    pub struct HistoryControls {
        #[template_child] pub back_button:    gtk::TemplateChild<gtk::Button>,
        #[template_child] pub forward_button: gtk::TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryControls {
        const NAME: &'static str = "HistoryControls";
        type Type = super::HistoryControls;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for HistoryControls {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("navigate-back").build(),
                    Signal::builder("navigate-forward").build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for HistoryControls {}
    impl BoxImpl for HistoryControls {}

    #[gtk::template_callbacks]
    impl HistoryControls {
        #[template_callback]
        fn on_back_clicked(&self) {
            self.obj().emit_by_name::<()>("navigate-back", &[]);
        }

        #[template_callback]
        fn on_forward_clicked(&self) {
            self.obj().emit_by_name::<()>("navigate-forward", &[]);
        }
    }
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct HistoryControls(ObjectSubclass<imp::HistoryControls>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
                    gtk::Orientable;
}

impl HistoryControls {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update button sensitivity based on stack sizes.
    pub fn set_sensitivity(&self, can_go_back: bool, can_go_forward: bool) {
        let imp = self.imp();
        imp.back_button.set_sensitive(can_go_back);
        imp.forward_button.set_sensitive(can_go_forward);
    }

    /// Reset both buttons to insensitive (e.g. when a new repo is loaded).
    pub fn reset(&self) {
        self.set_sensitivity(false, false);
    }
}

impl Default for HistoryControls {
    fn default() -> Self { Self::new() }
}

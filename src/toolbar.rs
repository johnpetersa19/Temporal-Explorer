/* toolbar.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! `TemporalToolbar` — the main application header bar extracted from
//! `window.blp` into its own reusable widget.
//!
//! This is a direct port of the Nautilus `NautilusToolbar` pattern:
//! the `Adw.HeaderBar` with all its children lives here so that
//! `window.blp` only needs `$TemporalToolbar toolbar {}` at the `[top]`
//! slot of the content `Adw.ToolbarView`.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, CompositeTemplate};
use adw::subclass::prelude::*;

use crate::history_controls::HistoryControls;
use crate::view_controls::ViewControls;

// ── GObject boilerplate ───────────────────────────────────────────────────────
mod imp {
    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/toolbar.ui")]
    pub struct TemporalToolbar {
        // Header bar itself (needed to call set_show_title_buttons, etc.)
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,

        // Start-slot buttons — forwarded to window.rs
        #[template_child]
        pub history_controls: TemplateChild<HistoryControls>,
        #[template_child]
        pub open_repo_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub show_sidebar_button: TemplateChild<gtk::ToggleButton>,

        // Title-slot: pathbar / location-entry switcher
        #[template_child]
        pub toolbar_switcher: TemplateChild<gtk::Stack>,
        #[template_child]
        pub address_bar: TemplateChild<gtk::Box>,
        #[template_child]
        pub location_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub location_cancel_btn: TemplateChild<gtk::Button>,

        // End-slot widgets
        #[template_child]
        pub view_controls: TemplateChild<ViewControls>,
        #[template_child]
        pub main_menu_button: TemplateChild<gtk::MenuButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TemporalToolbar {
        const NAME: &'static str = "TemporalToolbar";
        type Type = super::TemporalToolbar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            HistoryControls::ensure_type();
            ViewControls::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for TemporalToolbar {}
    impl WidgetImpl for TemporalToolbar {}
    impl BinImpl for TemporalToolbar {}
}

glib::wrapper! {
    pub struct TemporalToolbar(ObjectSubclass<imp::TemporalToolbar>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TemporalToolbar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    // ── Convenience accessors used by window.rs ──────────────────────────────

    pub fn history_controls(&self) -> &HistoryControls {
        &self.imp().history_controls
    }

    pub fn open_repo_button(&self) -> &gtk::Button {
        &self.imp().open_repo_button
    }

    pub fn show_sidebar_button(&self) -> &gtk::ToggleButton {
        &self.imp().show_sidebar_button
    }

    pub fn toolbar_switcher(&self) -> &gtk::Stack {
        &self.imp().toolbar_switcher
    }

    pub fn address_bar(&self) -> &gtk::Box {
        &self.imp().address_bar
    }

    pub fn location_entry(&self) -> &gtk::Entry {
        &self.imp().location_entry
    }

    pub fn location_cancel_btn(&self) -> &gtk::Button {
        &self.imp().location_cancel_btn
    }

    pub fn view_controls(&self) -> &ViewControls {
        &self.imp().view_controls
    }

    /// Switch from pathbar to the inline location-entry, or back.
    pub fn set_location_mode(&self, location_mode: bool) {
        let page = if location_mode { "location" } else { "pathbar" };
        self.imp().toolbar_switcher.set_visible_child_name(page);
        if location_mode {
            self.imp().location_entry.grab_focus();
        }
    }

    pub fn is_location_mode(&self) -> bool {
        self.imp()
            .toolbar_switcher
            .visible_child_name()
            .as_deref()
            == Some("location")
    }
}

impl Default for TemporalToolbar {
    fn default() -> Self {
        Self::new()
    }
}

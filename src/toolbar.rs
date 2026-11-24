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
//!
//! ## Blueprint ↔ Rust mapping
//!
//! Every widget declared with an `id` in `toolbar.blp` **must** appear
//! here as a `#[template_child]` so the template can be bound at
//! construction time.  Adding a widget to the `.blp` without a matching
//! `TemplateChild` field silently leaves it unwired (no panic, but the
//! widget is invisible to Rust code).

use adw::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{glib, CompositeTemplate};
use std::cell::Cell;
use std::sync::OnceLock;

use crate::history_controls::HistoryControls;
use crate::view_controls::ViewControls;

// ── GObject boilerplate ───────────────────────────────────────────────────────
mod imp {
    use super::*;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/toolbar.ui")]
    pub struct TemporalToolbar {
        // Header bar itself (needed to call set_show_title_buttons, etc.)
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,

        // ── Start-slot buttons ────────────────────────────────────────────────
        #[template_child]
        pub history_controls: TemplateChild<HistoryControls>,

        /// "Open Repository" button — wired in window.rs via connect_clicked.
        #[template_child]
        pub open_repo_button: TemplateChild<gtk::Button>,

        /// "New Branch" button — declared in toolbar.blp with
        /// `action-name: "win.new-branch"` so it fires the GAction directly.
        /// Exposed here so callers can also connect signals if needed.
        #[template_child]
        pub new_branch_button: TemplateChild<gtk::Button>,

        /// Sidebar toggle — bound bidirectionally to `split_view.show-sidebar`
        /// in window.rs `setup_callbacks`.
        #[template_child]
        pub show_sidebar_button: TemplateChild<gtk::ToggleButton>,

        // ── Title-slot: pathbar / location-entry switcher ─────────────────────
        #[template_child]
        pub toolbar_switcher: TemplateChild<gtk::Stack>,
        #[template_child]
        pub address_bar_scrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub address_bar: TemplateChild<gtk::Box>,
        #[template_child]
        pub location_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub location_cancel_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub folder_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub folder_search_close_btn: TemplateChild<gtk::Button>,

        // ── End-slot widgets ──────────────────────────────────────────────────
        #[template_child]
        pub search_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub view_controls: TemplateChild<ViewControls>,

        /// Visibility flags controlled by the `show-view-controls` and
        /// `show-history-controls` properties (driven by the window's
        /// `Adw.Breakpoint`, exactly like the Nautilus toolbar).
        pub show_view_controls: Cell<bool>,
        pub show_history_controls: Cell<bool>,
        /// Visibility state of the sidebar toggle button (distinct from the
        /// `TemplateChild` named `show_sidebar_button`).
        pub sidebar_button_shown: Cell<bool>,
    }

    impl Default for TemporalToolbar {
        fn default() -> Self {
            Self {
                header_bar: TemplateChild::default(),
                history_controls: TemplateChild::default(),
                open_repo_button: TemplateChild::default(),
                new_branch_button: TemplateChild::default(),
                show_sidebar_button: TemplateChild::default(),
                toolbar_switcher: TemplateChild::default(),
                address_bar_scrolled: TemplateChild::default(),
                address_bar: TemplateChild::default(),
                location_entry: TemplateChild::default(),
                location_cancel_btn: TemplateChild::default(),
                folder_search_entry: TemplateChild::default(),
                folder_search_close_btn: TemplateChild::default(),
                search_button: TemplateChild::default(),
                view_controls: TemplateChild::default(),
                // Same defaults as the GObject properties: the history and
                // view controls start visible and the sidebar toggle starts
                // hidden (the sidebar is always visible on wide windows, as
                // in the Nautilus).
                show_view_controls: Cell::new(true),
                show_history_controls: Cell::new(true),
                sidebar_button_shown: Cell::new(false),
            }
        }
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

    impl ObjectImpl for TemporalToolbar {
        fn constructed(&self) {
            self.parent_constructed();

            // Applies the visibility flags that were set before the template
            // was inflated (the properties may be set from the BLP).
            let obj = self.obj();
            obj.set_view_controls_visible(obj.view_controls_visible());
            obj.set_history_controls_visible(obj.history_controls_visible());
            obj.set_sidebar_button_visible(obj.sidebar_button_visible());
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecBoolean::builder("show-view-controls")
                        .default_value(true)
                        .build(),
                    glib::ParamSpecBoolean::builder("show-history-controls")
                        .default_value(true)
                        .build(),
                    glib::ParamSpecBoolean::builder("show-sidebar-button")
                        .default_value(false)
                        .build(),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "show-view-controls" => {
                    self.obj().set_view_controls_visible(value.get().unwrap());
                }
                "show-history-controls" => {
                    self.obj().set_history_controls_visible(value.get().unwrap());
                }
                "show-sidebar-button" => {
                    self.obj().set_sidebar_button_visible(value.get().unwrap());
                }
                _ => unimplemented!("unknown property: {}", pspec.name()),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "show-view-controls" => self.obj().view_controls_visible().to_value(),
                "show-history-controls" => self.obj().history_controls_visible().to_value(),
                "show-sidebar-button" => self.obj().sidebar_button_visible().to_value(),
                _ => unimplemented!("unknown property: {}", pspec.name()),
            }
        }
    }
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

    /// The "New Branch" button.
    ///
    /// Its `action-name` is already set to `"win.new-branch"` in the
    /// Blueprint, so clicking it fires the action automatically.  This
    /// accessor is provided for cases where additional signal wiring is
    /// needed (e.g. disabling the button when no repository is loaded).
    pub fn new_branch_button(&self) -> &gtk::Button {
        &self.imp().new_branch_button
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

    pub fn address_bar_scrolled(&self) -> &gtk::ScrolledWindow {
        &self.imp().address_bar_scrolled
    }

    pub fn location_entry(&self) -> &gtk::Entry {
        &self.imp().location_entry
    }

    pub fn location_cancel_btn(&self) -> &gtk::Button {
        &self.imp().location_cancel_btn
    }

    pub fn folder_search_entry(&self) -> &gtk::SearchEntry {
        &self.imp().folder_search_entry
    }

    pub fn folder_search_close_btn(&self) -> &gtk::Button {
        &self.imp().folder_search_close_btn
    }

    pub fn set_search_mode(&self, active: bool) {
        let page = if active { "search" } else { "pathbar" };
        self.imp().toolbar_switcher.set_visible_child_name(page);
        if active {
            self.imp().folder_search_entry.grab_focus();
        } else {
            self.imp().folder_search_entry.set_text("");
        }
    }

    pub fn search_button(&self) -> &gtk::ToggleButton {
        &self.imp().search_button
    }

    pub fn view_controls(&self) -> &ViewControls {
        &self.imp().view_controls
    }

    /// Show or hide the view controls (list/grid switcher).
    pub fn set_view_controls_visible(&self, visible: bool) {
        let imp = self.imp();
        imp.show_view_controls.set(visible);
        if imp.view_controls.is_bound() {
            imp.view_controls.set_visible(visible);
        }
    }

    pub fn view_controls_visible(&self) -> bool {
        self.imp().show_view_controls.get()
    }

    /// Show or hide the back/forward navigation controls.
    pub fn set_history_controls_visible(&self, visible: bool) {
        let imp = self.imp();
        imp.show_history_controls.set(visible);
        if imp.history_controls.is_bound() {
            imp.history_controls.set_visible(visible);
        }
    }

    pub fn history_controls_visible(&self) -> bool {
        self.imp().show_history_controls.get()
    }

    /// Show or hide the sidebar toggle button.
    ///
    /// The Nautilus hides this button on wide windows (the sidebar is
    /// always visible there) and reveals it only at the 682sp breakpoint.
    pub fn set_sidebar_button_visible(&self, visible: bool) {
        let imp = self.imp();
        imp.sidebar_button_shown.set(visible);
        if imp.show_sidebar_button.is_bound() {
            imp.show_sidebar_button.set_visible(visible);
        }
    }

    pub fn sidebar_button_visible(&self) -> bool {
        self.imp().sidebar_button_shown.get()
    }

    /// Switch from pathbar to the inline location-entry, or back.
    pub fn set_location_mode(&self, location_mode: bool) {
        let page = if location_mode { "location" } else { "pathbar" };
        self.imp().toolbar_switcher.set_visible_child_name(page);

        if location_mode {
            self.imp().search_button.set_active(false);
            // Caps the location entry at 88 characters, as the Nautilus
            // does.  Applied only in location mode so that the path bar is
            // not widened while it is the visible page.
            self.imp().location_entry.set_max_width_chars(88);
            self.imp().location_entry.grab_focus();
        } else {
            self.imp().location_entry.set_max_width_chars(-1);
        }
    }

    pub fn is_location_mode(&self) -> bool {
        self.imp().toolbar_switcher.visible_child_name().as_deref() == Some("location")
    }
}

impl Default for TemporalToolbar {
    fn default() -> Self {
        Self::new()
    }
}

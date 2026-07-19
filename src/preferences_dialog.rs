/* preferences_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! `PreferencesDialog` — application settings.
//!
//! Three pages:
//! * **General** — date format (relative/short/ISO), commits per page,
//!                 default branch, shallow clone depth, fetch on open.
//! * **Appearance** — default view (list/grid), author avatars, dense mode,
//!                    file sort, grid zoom/captions, branch graph,
//!                    diff syntax highlight, word diff, context lines.
//! * **Advanced** — verify signatures, follow renames, submodules,
//!                  background fetch + interval.
//!
//! All settings are backed by `gio::Settings` (schema
//! `io.github.TemporalExplorer`).  On construction, current
//! values are loaded from GSettings; on change, they are written back.
//!
//! # Usage
//! ```rust
//! let prefs = PreferencesDialog::new(&settings);
//! prefs.present(Some(&window));
//! ```

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gio;
use gtk::glib;

// ── GObject subclass ───────────────────────────────────────────────────────────

mod imp {
    use super::*;
    use std::cell::OnceCell;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/preferences-dialog.ui")]
    pub struct PreferencesDialog {
        // ── General page ──
        #[template_child]
        pub fetch_on_open_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub commits_per_page_row: TemplateChild<adw::SpinRow>,
        #[template_child]
        pub default_branch_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub clone_depth_row: TemplateChild<adw::SpinRow>,
        #[template_child]
        pub date_format_relative_button: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub date_format_short_button: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub date_format_iso_button: TemplateChild<gtk::CheckButton>,

        // ── Appearance page ──
        #[template_child]
        pub default_view_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub grid_zoom_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub file_sort_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub show_hidden_files_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub caption_status_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub caption_extension_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub caption_size_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub caption_date_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub show_avatars_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub dense_mode_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub show_graph_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub syntax_highlight_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub word_diff_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub context_lines_row: TemplateChild<adw::SpinRow>,

        // ── Advanced page ──
        #[template_child]
        pub verify_signatures_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub follow_renames_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub include_submodules_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub background_fetch_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub background_fetch_interval_row: TemplateChild<adw::SpinRow>,

        pub settings: OnceCell<gio::Settings>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PreferencesDialog {
        const NAME: &'static str = "PreferencesDialog";
        type Type = super::PreferencesDialog;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PreferencesDialog {
        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for PreferencesDialog {}
    impl AdwDialogImpl for PreferencesDialog {}
    impl PreferencesDialogImpl for PreferencesDialog {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct PreferencesDialog(ObjectSubclass<imp::PreferencesDialog>)
        @extends adw::PreferencesDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PreferencesDialog {
    pub fn new(settings: &gio::Settings) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().settings.set(settings.clone()).ok();
        obj.bind_settings();
        obj
    }

    // ── GSettings ↔ widget bindings ────────────────────────────────────────

    fn bind_settings(&self) {
        let imp = self.imp();
        let s = imp.settings.get().unwrap();

        // ── General ──────────────────────────────────────────────────────
        s.bind("fetch-on-open", &*imp.fetch_on_open_row, "active")
            .build();
        s.bind("commits-per-page", &*imp.commits_per_page_row, "value")
            .build();
        s.bind("default-branch", &*imp.default_branch_row, "text")
            .build();
        s.bind("clone-depth", &*imp.clone_depth_row, "value")
            .build();

        // Date format radio buttons use action-name/action-target in the
        // .blp so they are driven by the "preferences.date-format" action.
        // Here we just install the action pointing to GSettings.
        let action_group = gio::SimpleActionGroup::new();
        let date_fmt_action = gio::SimpleAction::new_stateful(
            "date-format",
            Some(&String::static_variant_type()),
            &s.string("date-format").to_variant(),
        );
        {
            let s = s.clone();
            date_fmt_action.connect_activate(move |action, param| {
                if let Some(v) = param {
                    let fmt: String = v.get().unwrap_or_default();
                    s.set_string("date-format", &fmt).ok();
                    action.set_state(v);
                }
            });
        }
        action_group.add_action(&date_fmt_action);
        self.insert_action_group("preferences", Some(&action_group));

        // ── Appearance ───────────────────────────────────────────────────
        s.bind("show-hidden-files", &*imp.show_hidden_files_row, "active")
            .build();
        s.bind("show-avatars", &*imp.show_avatars_row, "active")
            .build();
        s.bind("dense-mode", &*imp.dense_mode_row, "active").build();
        s.bind("show-graph", &*imp.show_graph_row, "active").build();
        s.bind("syntax-highlight", &*imp.syntax_highlight_row, "active")
            .build();
        s.bind("word-diff", &*imp.word_diff_row, "active").build();
        s.bind("context-lines", &*imp.context_lines_row, "value")
            .build();

        // default-view combo: 0 = list, 1 = grid
        let view_action = gio::SimpleAction::new_stateful(
            "default-view",
            Some(&String::static_variant_type()),
            &s.string("default-view").to_variant(),
        );
        {
            let s = s.clone();
            view_action.connect_activate(move |action, param| {
                if let Some(v) = param {
                    let view: String = v.get().unwrap_or_default();
                    s.set_string("default-view", &view).ok();
                    action.set_state(v);
                }
            });
        }
        action_group.add_action(&view_action);

        // Sync ComboRow to GSettings string via index
        {
            let row = imp.default_view_row.get();
            let idx: u32 = if s.string("default-view") == "grid" {
                1
            } else {
                0
            };
            row.set_selected(idx);
            let s2 = s.clone();
            row.connect_selected_notify(move |r| {
                let val = if r.selected() == 1 { "grid" } else { "list" };
                s2.set_string("default-view", val).ok();
            });
        }

        {
            let row = imp.grid_zoom_row.get();
            row.set_selected(s.uint("grid-zoom-level").min(2));
            let s2 = s.clone();
            row.connect_selected_notify(move |r| {
                s2.set_uint("grid-zoom-level", r.selected().min(2)).ok();
            });
        }

        {
            let row = imp.file_sort_row.get();
            row.set_selected(match s.string("file-sort-mode").as_str() {
                "name-desc" => 1,
                "last-modified" => 2,
                "first-modified" => 3,
                "size" => 4,
                "type" => 5,
                _ => 0,
            });
            let s2 = s.clone();
            row.connect_selected_notify(move |r| {
                let key = match r.selected() {
                    1 => "name-desc",
                    2 => "last-modified",
                    3 => "first-modified",
                    4 => "size",
                    5 => "type",
                    _ => "name",
                };
                s2.set_string("file-sort-mode", key).ok();
            });
        }

        self.bind_caption_flags();

        // ── Advanced ─────────────────────────────────────────────────────
        s.bind("verify-signatures", &*imp.verify_signatures_row, "active")
            .build();
        s.bind("follow-renames", &*imp.follow_renames_row, "active")
            .build();
        s.bind("include-submodules", &*imp.include_submodules_row, "active")
            .build();
        s.bind("background-fetch", &*imp.background_fetch_row, "active")
            .build();
        s.bind(
            "background-fetch-interval",
            &*imp.background_fetch_interval_row,
            "value",
        )
        .build();

        // Show/hide interval row based on background-fetch switch
        {
            let interval_row = imp.background_fetch_interval_row.get();
            let bg_row = imp.background_fetch_row.get();
            interval_row.set_sensitive(bg_row.is_active());
            bg_row.connect_active_notify(move |r| {
                interval_row.set_sensitive(r.is_active());
            });
        }
    }

    fn bind_caption_flags(&self) {
        const STATUS: u32 = 0b0001;
        const EXTENSION: u32 = 0b0010;
        const SIZE: u32 = 0b0100;
        const DATE: u32 = 0b1000;

        let imp = self.imp();
        let s = imp.settings.get().unwrap();
        let flags = s.uint("grid-caption-flags");

        imp.caption_status_row.set_active(flags & STATUS != 0);
        imp.caption_extension_row.set_active(flags & EXTENSION != 0);
        imp.caption_size_row.set_active(flags & SIZE != 0);
        imp.caption_date_row.set_active(flags & DATE != 0);

        self.connect_caption_row(&imp.caption_status_row, STATUS);
        self.connect_caption_row(&imp.caption_extension_row, EXTENSION);
        self.connect_caption_row(&imp.caption_size_row, SIZE);
        self.connect_caption_row(&imp.caption_date_row, DATE);
    }

    fn connect_caption_row(&self, row: &adw::SwitchRow, bit: u32) {
        let settings = self.imp().settings.get().unwrap().clone();

        row.connect_active_notify(move |row| {
            let mut flags = settings.uint("grid-caption-flags");
            if row.is_active() {
                flags |= bit;
            } else {
                flags &= !bit;
            }
            settings.set_uint("grid-caption-flags", flags).ok();
        });
    }
}

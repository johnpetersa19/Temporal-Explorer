/* window.rs
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

//! Main application window.
//!
//! This module is intentionally kept as an *orchestrator*: it owns the
//! GObject subclass boilerplate, wires GTK signals, and delegates all
//! widget-construction work to the purpose-built sub-modules:
//!
//! | Responsibility              | Module                        |
//! |-----------------------------|-------------------------------|
//! | List-view widget building   | [`crate::views::list_view`]   |
//! | Grid-view widget building   | [`crate::views::grid_view`]   |
//! | Address-bar rebuilding      | [`crate::address_bar`]        |
//! | Commit list / search        | [`crate::commit_controller`]  |
//! | File content preview dialog | [`crate::file_preview`]       |
//! | Git history / tree reads    | [`crate::git_engine`]         |
//! | Timeline grouping logic     | [`crate::timeline_filter`]    |
//!
//! ## Styles
//!
//! All application CSS is stored in `src/temporal-explorer.css` and loaded
//! via GResource in `setup_styles()`.  No CSS strings live in Rust code.
//!
//! ## Right-panel layout
//!
//! `content_toolbar_view` has a permanent `Stack` child (`right_panel_stack`)
//! with two named pages:
//!
//! ```
//! right_panel_stack
//!   ├── "empty"   → adw::StatusPage (never re-parented)
//!   └── "content" → gtk::Box right_panel_content  (dynamic views appended here)
//! ```
//!
//! `replace_right_panel` clears `right_panel_content`, appends the new widget,
//! and flips the stack to "content".  `show_empty_state` clears the box and
//! flips back to "empty".  `set_content()` on `content_toolbar_view` is never
//! called at runtime, so the "parent must be NULL" assertion can never fire.
//!
//! ## Timeline navigation
//!
//! The left sidebar is a 3-level drill-down:
//!
//! ```
//! years  →  months (for selected year)  →  commits (for selected month)
//! ```
//!
//! The active level is tracked by [`TimelineLevel`] stored in
//! `imp.timeline_level`.  The back button (`timeline_back_button`) pops one
//! level; the `timeline_stack` `Stack` slides between the three pages.
//!
//! ## Search scope
//!
//! The search (`on_search_changed`) filters the `all_commits` in-memory
//! cache.  When a year is selected (`selected_year != 0`) the results are
//! scoped to that year.  When no year is selected (Years screen or after
//! pressing Back to the years level) the search spans all commits.

use adw::prelude::AdwApplicationWindowExt;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use gtk::prelude::*;
use glib::object::ObjectExt;
use gettextrs::gettext;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crate::address_bar;
use crate::git_engine::{CommitInfo, DirCache, HistoryReader, SnapshotResolver, TreeNode};
use crate::commit_controller;
use crate::file_preview;
use crate::timeline_filter;
use crate::views::{list_view, grid_view};

// ── ViewMode ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode { #[default] List, Grid }

// ── TimelineLevel ──────────────────────────────────────────────────────────────────────────

/// Which page of the sidebar `timeline_stack` is currently visible.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum TimelineLevel {
    #[default]
    Years,
    Months,
    Commits,
}

// ── DebugRepository ─────────────────────────────────────────────────────────────────────────

pub struct DebugRepository(pub git2::Repository);

impl std::fmt::Debug for DebugRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Repository").field(&"<git2::Repository>").finish()
    }
}

impl std::ops::Deref for DebugRepository {
    type Target = git2::Repository;
    fn deref(&self) -> &Self::Target { &self.0 }
}

// ── Private implementation ────────────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/window.ui")]
    pub struct TemporalExplorerWindow {
        // Header
        #[template_child] pub open_repo_button:     TemplateChild<gtk::Button>,
        #[template_child] pub nav_back_button:      TemplateChild<gtk::Button>,
        #[template_child] pub nav_forward_button:   TemplateChild<gtk::Button>,
        #[template_child] pub view_toggle_button:   TemplateChild<gtk::Button>,
        #[template_child] pub show_sidebar_button:  TemplateChild<gtk::ToggleButton>,
        #[template_child] pub window_title:         TemplateChild<adw::WindowTitle>,

        // Nautilus-style toolbar_switcher Stack
        #[template_child] pub toolbar_switcher:     TemplateChild<gtk::Stack>,
        #[template_child] pub address_bar:          TemplateChild<gtk::Box>,
        #[template_child] pub location_entry:       TemplateChild<gtk::Entry>,
        #[template_child] pub location_cancel_btn:  TemplateChild<gtk::Button>,

        // Left panel — timeline drill-down
        #[template_child] pub timeline_stack:        TemplateChild<gtk::Stack>,
        #[template_child] pub timeline_back_button:  TemplateChild<gtk::Button>,
        #[template_child] pub timeline_header_title: TemplateChild<adw::WindowTitle>,
        #[template_child] pub year_list:             TemplateChild<gtk::ListBox>,
        #[template_child] pub month_list:            TemplateChild<gtk::ListBox>,
        #[template_child] pub commit_search_entry:   TemplateChild<gtk::SearchEntry>,
        #[template_child] pub commit_list:           TemplateChild<gtk::ListBox>,

        // Right panel — permanent stack (never re-parented)
        #[template_child] pub content_toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child] pub right_panel_stack:    TemplateChild<gtk::Stack>,
        #[template_child] pub right_panel_content:  TemplateChild<gtk::Box>,
        #[template_child] pub empty_state:          TemplateChild<adw::StatusPage>,
        #[template_child] pub split_view:           TemplateChild<adw::OverlaySplitView>,

        // Bottom bar
        #[template_child] pub commit_info_bar:      TemplateChild<gtk::ActionBar>,
        #[template_child] pub commit_hash_label:    TemplateChild<gtk::Label>,
        #[template_child] pub commit_message_label: TemplateChild<gtk::Label>,
        #[template_child] pub commit_date_label:    TemplateChild<gtk::Label>,

        // Runtime state
        pub all_commits:      RefCell<Vec<CommitInfo>>,
        pub repo_path:        RefCell<Option<PathBuf>>,
        pub repository:       RefCell<Option<DebugRepository>>,
        pub last_query:       RefCell<String>,
        pub current_hash:     RefCell<Option<String>>,
        pub current_dir:      RefCell<PathBuf>,
        pub history_back:     RefCell<Vec<PathBuf>>,
        pub history_forward:  RefCell<Vec<PathBuf>>,
        pub view_mode:        RefCell<ViewMode>,
        pub repo_name:        RefCell<String>,

        pub timeline_level:   RefCell<TimelineLevel>,
        pub selected_year:    Cell<i32>,
        pub loading_commits:  Cell<bool>,
        pub dir_cache:        RefCell<DirCache>,
        pub search_debounce:  RefCell<Option<glib::SourceId>>,
        pub search_cancel:    RefCell<Option<Arc<AtomicBool>>>,
        pub load_cancel:      RefCell<Option<Arc<AtomicBool>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TemporalExplorerWindow {
        const NAME: &'static str = "TemporalExplorerWindow";
        type Type = super::TemporalExplorerWindow;
        type ParentType = adw::ApplicationWindow;
        fn class_init(klass: &mut Self::Class) { klass.bind_template(); }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) { obj.init_template(); }
    }

    impl ObjectImpl for TemporalExplorerWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_callbacks();
            self.obj().setup_styles();
        }
    }
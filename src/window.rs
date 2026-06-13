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

//! Main application window — orchestrator module.
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

use adw::prelude::{AdwDialogExt, AlertDialogExt};
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
use crate::search_filter_popover::{SearchFilterPopover, FilterState};
use crate::timeline_filter;
use crate::views::{list_view, grid_view};
use crate::views::list_view::{OnEnterDir, OnOpenFile};

// ── ViewMode ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode { #[default] List, Grid }

// ── TimelineLevel ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum TimelineLevel {
    #[default] Years,
    Months,
    Commits,
}

// ── DebugRepository ────────────────────────────────────────────────────────────

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

// ── Private implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/window.ui")]
    pub struct TemporalExplorerWindow {
        #[template_child] pub open_repo_button:     TemplateChild<gtk::Button>,
        #[template_child] pub nav_back_button:      TemplateChild<gtk::Button>,
        #[template_child] pub nav_forward_button:   TemplateChild<gtk::Button>,
        #[template_child] pub view_toggle_button:   TemplateChild<gtk::Button>,
        #[template_child] pub show_sidebar_button:  TemplateChild<gtk::ToggleButton>,
        #[template_child] pub window_title:         TemplateChild<adw::WindowTitle>,

        #[template_child] pub toolbar_switcher:     TemplateChild<gtk::Stack>,
        #[template_child] pub address_bar:          TemplateChild<gtk::Box>,
        #[template_child] pub location_entry:       TemplateChild<gtk::Entry>,
        #[template_child] pub location_cancel_btn:  TemplateChild<gtk::Button>,

        #[template_child] pub timeline_stack:        TemplateChild<gtk::Stack>,
        #[template_child] pub timeline_back_button:  TemplateChild<gtk::Button>,
        #[template_child] pub timeline_header_title: TemplateChild<adw::WindowTitle>,
        #[template_child] pub year_list:             TemplateChild<gtk::ListBox>,
        #[template_child] pub month_list:            TemplateChild<gtk::ListBox>,
        #[template_child] pub commit_search_entry:   TemplateChild<gtk::SearchEntry>,
        #[template_child] pub commit_list:           TemplateChild<gtk::ListBox>,

        // ── Filter button (injected into the toolbar at runtime) ───────────────
        pub filter_button:  RefCell<Option<gtk::ToggleButton>>,
        pub filter_popover: RefCell<Option<SearchFilterPopover>>,

        #[template_child] pub content_toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child] pub right_panel_stack:    TemplateChild<gtk::Stack>,
        #[template_child] pub right_panel_content:  TemplateChild<gtk::Box>,
        #[template_child] pub empty_state:          TemplateChild<adw::StatusPage>,
        #[template_child] pub split_view:           TemplateChild<adw::OverlaySplitView>,

        #[template_child] pub commit_info_bar:      TemplateChild<gtk::ActionBar>,
        #[template_child] pub commit_hash_label:    TemplateChild<gtk::Label>,
        #[template_child] pub commit_message_label: TemplateChild<gtk::Label>,
        #[template_child] pub commit_date_label:    TemplateChild<gtk::Label>,

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
        pub search_debounce:  RefCell<Option<Arc<AtomicBool>>>,
        pub search_cancel:    RefCell<Option<Arc<AtomicBool>>>,

        pub filter_state:     RefCell<FilterState>,
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

    impl WidgetImpl for TemporalExplorerWindow {}
    impl WindowImpl for TemporalExplorerWindow {}
    impl ApplicationWindowImpl for TemporalExplorerWindow {}
    impl AdwApplicationWindowImpl for TemporalExplorerWindow {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct TemporalExplorerWindow(ObjectSubclass<imp::TemporalExplorerWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements
            gio::ActionGroup, gio::ActionMap,
            gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

// ── Free helpers ───────────────────────────────────────────────────────────────

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

// ── Calendar / date matching helpers ──────────────────────────────────────────

/// Returns `true` if `query` matches any date-related field of the commit's
/// timestamp: ISO date (`2026-03-15`), short date (`2026-03`), year (`2026`),
/// full/abbreviated month name (locale-independent English), or the
/// human-readable date-time string shown in the UI (`2026-03-15 14:32`).
///
/// All comparisons are case-insensitive.
fn matches_calendar(ts: i64, q: &str) -> bool {
    let Ok(dt) = glib::DateTime::from_unix_local(ts) else { return false };

    // Build candidate strings once
    let year  = dt.year();
    let month = dt.month() as u32;
    let day   = dt.day_of_month();

    // ISO date:  "2026-03-15"
    let iso_date = format!("{:04}-{:02}-{:02}", year, month, day);
    // Year-month: "2026-03"
    let year_month = format!("{:04}-{:02}", year, month);
    // Year only:  "2026"
    let year_str = format!("{:04}", year);
    // Human datetime: "2026-03-15 14:32"
    let human = dt
        .format("%Y-%m-%d %H:%M")
        .map(|s| s.to_string())
        .unwrap_or_default();

    // English month names (full + abbreviated) — constant, locale-independent
    let month_full = match month {
        1  => "january",   2  => "february", 3  => "march",
        4  => "april",     5  => "may",       6  => "june",
        7  => "july",      8  => "august",    9  => "september",
        10 => "october",   11 => "november",  12 => "december",
        _  => "",
    };
    let month_abbr = &month_full[..3.min(month_full.len())];

    // Translated month name via gettext (respects user locale)
    let month_translated = timeline_filter::month_name(month).to_lowercase();

    iso_date.contains(q)
        || year_month.contains(q)
        || year_str == q
        || human.contains(q)
        || month_full.contains(q)
        || month_abbr == q
        || month_translated.contains(q)
}

impl TemporalExplorerWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    // ── Callback wiring ────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        let win = self.clone();
        imp.open_repo_button.connect_clicked(move |_| { win.open_repo_dialog(); });

        let win = self.clone();
        imp.nav_back_button.connect_clicked(move |_| { win.navigate_back(); });

        let win = self.clone();
        imp.nav_forward_button.connect_clicked(move |_| { win.navigate_forward(); });

        let win = self.clone();
        imp.view_toggle_button.connect_clicked(move |_| { win.toggle_view_mode(); });

        let win_g = self.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_pressed(move |_, _n, _, _| { win_g.enter_location_mode(); });
        imp.address_bar.add_controller(gesture);

        let win = self.clone();
        imp.location_entry.connect_activate(move |entry| {
            win.navigate_to_typed_path(entry.text().as_str());
        });

        let win = self.clone();
        imp.location_cancel_btn.connect_clicked(move |_| { win.leave_location_mode(); });

        let win = self.clone();
        imp.timeline_back_button.connect_clicked(move |_| { win.timeline_pop(); });

        let win = self.clone();
        imp.year_list.connect_row_activated(move |_, row| {
            let year = unsafe {
                row.data::<i32>("year")
                    .map(|p| *p.as_ref())
                    .unwrap_or(row.index())
            };
            win.on_year_selected(year);
        });

        let win = self.clone();
        imp.month_list.connect_row_activated(move |_, row| {
            if let Some(m) = unsafe { row.data::<u32>("month") } {
                win.on_month_selected(unsafe { *m.as_ref() });
            }
        });

        let win = self.clone();
        imp.commit_list.connect_row_activated(move |_, row| {
            if let Some(h) = unsafe { row.data::<String>("hash") } {
                win.on_commit_selected(unsafe { h.as_ref().clone() });
            }
        });

        let win = self.clone();
        imp.commit_search_entry.connect_search_changed(move |entry| {
            win.on_search_changed(entry.text().to_string());
        });

        self.setup_filter_popover();
    }

    // ── CSS loader ────────────────────────────────────────────────────────────

    fn setup_styles(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_resource(
            "/io/github/johnpetersa19/TemporalExplorer/temporal-explorer.css",
        );
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display, &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    // ── Repository opening ─────────────────────────────────────────────────────

    fn open_repo_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Repository"))
            .modal(true)
            .build();
        let win = self.clone();
        dialog.select_folder(Some(self), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() { win.load_repository(path); }
            }
        });
    }

    pub fn load_repository(&self, path: PathBuf) {
        let cancel = Arc::new(AtomicBool::new(false));
        { if let Some(prev) = self.imp().load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }}
        *self.imp().load_cancel.borrow_mut() = Some(cancel.clone());

        match git2::Repository::open(&path) {
            Ok(repo) => {
                let repo_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repository")
                    .to_string();
                *self.imp().repo_path.borrow_mut() = Some(path.clone());
                *self.imp().repository.borrow_mut() = Some(DebugRepository(repo));
                *self.imp().repo_name.borrow_mut() = repo_name.clone();
                *self.imp().current_dir.borrow_mut() = PathBuf::new();
                self.imp().history_back.borrow_mut().clear();
                self.imp().history_forward.borrow_mut().clear();
                self.imp().window_title.set_title(&repo_name);
                self.imp().window_title.set_subtitle(path.to_str().unwrap_or(""));

                if let Some(ref pop) = *self.imp().filter_popover.borrow() {
                    if let Some(ref repo_wrapper) = *self.imp().repository.borrow() {
                        let branches: Vec<String> = repo_wrapper
                            .branches(Some(git2::BranchType::Local))
                            .map(|iter| {
                                iter.filter_map(|b| {
                                    b.ok().and_then(|(b, _)| {
                                        b.name().ok().flatten().map(|n| n.to_string())
                                    })
                                }).collect()
                            })
                            .unwrap_or_default();
                        pop.populate_branch_chips(&branches);
                    }
                }

                self.load_timeline(cancel);
            }
            Err(e) => self.show_error(&format!("{}: {e}", gettext("Failed to open repository"))),
        }
    }

    // ── Timeline loading ───────────────────────────────────────────────────────

    fn load_timeline(&self, _cancel: Arc<AtomicBool>) {
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };
        self.imp().loading_commits.set(true);
        self.show_empty_state();

        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<CommitInfo>, String>>(1);
        std::thread::spawn(move || {
            let result = HistoryReader::open(&repo_path)
                .and_then(|r| r.list_commits())
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });

        let win = self.clone();
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(commits) => {
                win.imp().loading_commits.set(false);
                match commits {
                    Ok(list) => {
                        if let Some(ref pop) = *win.imp().filter_popover.borrow() {
                            let mut seen = std::collections::HashSet::new();
                            let authors: Vec<String> = list.iter()
                                .map(|c| c.author.clone())
                                .filter(|a| seen.insert(a.clone()))
                                .collect();
                            pop.populate_author_chips(&authors);
                        }

                        *win.imp().all_commits.borrow_mut() = list;
                        win.populate_year_list();
                        win.imp().split_view.set_show_sidebar(true);
                    }
                    Err(e) => win.show_error(&format!("{}: {e}", gettext("Failed to read history"))),
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    // ── Year list ─────────────────────────────────────────────────────────────

    fn populate_year_list(&self) {
        let imp = self.imp();
        imp.year_list.remove_all();

        let commits = imp.all_commits.borrow();
        let years = timeline_filter::years_in_range(&commits);
        for (year, count) in &years {
            let row = commit_controller::build_year_row(*year, *count);
            imp.year_list.append(&row);
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title.set_subtitle("");
    }

    // ── Year selected ─────────────────────────────────────────────────────────

    fn on_year_selected(&self, year: i32) {
        let imp = self.imp();
        imp.selected_year.set(year);
        imp.month_list.remove_all();

        let commits = imp.all_commits.borrow();
        let months = timeline_filter::months_for_year(&commits, year);
        for (month, count) in &months {
            let row = commit_controller::build_month_row(*month, *count);
            imp.month_list.append(&row);
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
        imp.timeline_stack.set_visible_child_name("months");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(&year.to_string());
        imp.timeline_header_title.set_subtitle("");
    }

    // ── Month selected ────────────────────────────────────────────────────────

    fn on_month_selected(&self, month: u32) {
        let imp = self.imp();
        let year = imp.selected_year.get();
        imp.commit_list.remove_all();

        let commits = imp.all_commits.borrow().clone();
        let filtered = timeline_filter::commits_for_month(&commits, year, month);
        commit_controller::populate_commit_list(&imp.commit_list, &filtered);

        *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
        imp.timeline_stack.set_visible_child_name("commits");
        imp.timeline_header_title.set_subtitle(&format!(
            "{} {}",
            timeline_filter::month_name(month),
            year,
        ));
    }

    // ── Timeline back ─────────────────────────────────────────────────────────

    fn timeline_pop(&self) {
        let imp = self.imp();
        let current_level = { *imp.timeline_level.borrow() };

        match current_level {
            TimelineLevel::Commits => {
                *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
                imp.timeline_stack.set_visible_child_name("months");
                imp.timeline_header_title.set_subtitle("");
            }
            TimelineLevel::Months => {
                *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
                imp.timeline_stack.set_visible_child_name("years");
                imp.timeline_back_button.set_visible(false);
                imp.timeline_header_title.set_title(&gettext("Timeline"));
                imp.timeline_header_title.set_subtitle("");
                imp.selected_year.set(0);
            }
            TimelineLevel::Years => {}
        }
    }

    // ── Commit selected ───────────────────────────────────────────────────────

    fn on_commit_selected(&self, hash: String) {
        let imp = self.imp();
        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut() = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        {
            let commits = imp.all_commits.borrow();
            if let Some(commit) = commits.iter().find(|c| c.hash.starts_with(&hash[..7.min(hash.len())])) {
                imp.commit_hash_label.set_label(&commit.hash[..8.min(commit.hash.len())]);
                imp.commit_message_label.set_label(&commit.summary);
                imp.commit_date_label.set_label(&Self::format_timestamp(commit.timestamp));
                imp.commit_info_bar.set_revealed(true);
            }
        }
        self.navigate_to_dir(PathBuf::new());
    }

    // ── Directory navigation ──────────────────────────────────────────────────

    pub fn navigate_to_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let hash = match imp.current_hash.borrow().clone() { Some(h) => h, None => return };
        let repo_path = match imp.repo_path.borrow().clone() { Some(p) => p, None => return };
        let repo_name = imp.repo_name.borrow().clone();

        *imp.current_dir.borrow_mut() = dir.clone();

        let win_ab1 = self.clone();
        let win_ab2 = self.clone();
        address_bar::rebuild_address_bar(
            &imp.address_bar.clone(),
            &repo_name,
            &dir,
            move |path: PathBuf| { win_ab1.push_dir(path); },
            move || { win_ab2.enter_location_mode(); },
        );
        self.update_nav_buttons();

        let cancel = Arc::new(AtomicBool::new(false));
        { if let Some(prev) = imp.load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }}
        *imp.load_cancel.borrow_mut() = Some(cancel);

        let dir_clone = dir.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<TreeNode>, String>>(1);
        std::thread::spawn(move || {
            let result = git2::Repository::open(&repo_path)
                .map_err(|e| e.to_string())
                .and_then(|repo| {
                    SnapshotResolver::new(&repo)
                        .resolve_dir(&hash, &dir_clone)
                        .map_err(|e| e.to_string())
                });
            let _ = tx.send(result);
        });

        let win = self.clone();
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(nodes) => win.render_dir(nodes),
                    Err(e) => win.show_error(&format!("{}: {e}", gettext("Error reading tree"))),
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    fn render_dir(&self, nodes: Vec<TreeNode>) {
        let imp = self.imp();
        let mode = *imp.view_mode.borrow();
        let hash = imp.current_hash.borrow().clone().unwrap_or_default();

        let win1 = self.clone();
        let win2 = self.clone();
        let on_enter_dir: OnEnterDir = Box::new(move |path: PathBuf| { win1.push_dir(path); });
        let on_open_file: OnOpenFile = Box::new(move |path: &std::path::Path, _h: &str| {
            win2.preview_file(path);
        });

        let widget: gtk::Widget = match mode {
            ViewMode::List =>
                list_view::build_list_view(&nodes, &hash, on_enter_dir, on_open_file).upcast(),
            ViewMode::Grid =>
                grid_view::build_grid_view(&nodes, &hash, on_enter_dir, on_open_file).upcast(),
        };
        self.replace_right_panel(widget);
    }

    // ── Right-panel management ────────────────────────────────────────────────

    pub fn replace_right_panel(&self, widget: gtk::Widget) {
        let imp = self.imp();
        clear_box(&imp.right_panel_content);
        imp.right_panel_content.append(&widget);
        imp.right_panel_stack.set_visible_child_name("content");
    }

    pub fn show_empty_state(&self) {
        let imp = self.imp();
        clear_box(&imp.right_panel_content);
        imp.right_panel_stack.set_visible_child_name("empty");
    }

    // ── File preview ──────────────────────────────────────────────────────────

    pub fn preview_file(&self, path: &std::path::Path) {
        let hash = match self.imp().current_hash.borrow().clone() { Some(h) => h, None => return };
        let repo_path = match self.imp().repo_path.borrow().clone() { Some(p) => p, None => return };

        match git2::Repository::open(&repo_path) {
            Ok(repo) => {
                file_preview::show_file_preview(self, &repo, &hash, path);
            }
            Err(e) => self.show_error(&format!("{}: {e}", gettext("Cannot open repository"))),
        }
    }

    // ── Navigation helpers ────────────────────────────────────────────────────

    pub fn push_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let prev = imp.current_dir.borrow().clone();
        { imp.history_back.borrow_mut().push(prev); }
        { imp.history_forward.borrow_mut().clear(); }
        self.navigate_to_dir(dir);
    }

    fn navigate_back(&self) {
        let imp = self.imp();
        let dir = { imp.history_back.borrow_mut().pop() };
        if let Some(dir) = dir {
            let cur = { imp.current_dir.borrow().clone() };
            { imp.history_forward.borrow_mut().push(cur); }
            self.navigate_to_dir(dir);
        }
    }

    fn navigate_forward(&self) {
        let imp = self.imp();
        let dir = { imp.history_forward.borrow_mut().pop() };
        if let Some(dir) = dir {
            let cur = { imp.current_dir.borrow().clone() };
            { imp.history_back.borrow_mut().push(cur); }
            self.navigate_to_dir(dir);
        }
    }

    fn update_nav_buttons(&self) {
        let imp = self.imp();
        if let Ok(back) = imp.history_back.try_borrow() {
            imp.nav_back_button.set_sensitive(!back.is_empty());
        }
        if let Ok(fwd) = imp.history_forward.try_borrow() {
            imp.nav_forward_button.set_sensitive(!fwd.is_empty());
        }
    }

    // ── Location bar ──────────────────────────────────────────────────────────

    pub fn enter_location_mode(&self) {
        let imp = self.imp();
        let current = { imp.current_dir.borrow().clone() };
        imp.location_entry.set_text(current.to_str().unwrap_or(""));
        imp.location_entry.grab_focus();
        imp.toolbar_switcher.set_visible_child_name("location");
    }

    fn leave_location_mode(&self) {
        self.imp().toolbar_switcher.set_visible_child_name("pathbar");
    }

    fn navigate_to_typed_path(&self, text: &str) {
        self.leave_location_mode();
        self.push_dir(PathBuf::from(text.trim()));
    }

    // ── View mode toggle ──────────────────────────────────────────────────────

    fn toggle_view_mode(&self) {
        let imp = self.imp();
        let current_mode = { *imp.view_mode.borrow() };

        let new_mode = match current_mode {
            ViewMode::List => {
                imp.view_toggle_button.set_icon_name("view-list-symbolic");
                ViewMode::Grid
            }
            ViewMode::Grid => {
                imp.view_toggle_button.set_icon_name("view-grid-symbolic");
                ViewMode::List
            }
        };

        { *imp.view_mode.borrow_mut() = new_mode; }

        let dir = imp.current_dir.borrow().clone();
        let has_hash = { imp.current_hash.borrow().is_some() };
        if has_hash { self.navigate_to_dir(dir); }
    }

    // ── Search ────────────────────────────────────────────────────────────────

    fn on_search_changed(&self, query: String) {
        let imp = self.imp();

        if let Some(prev) = imp.search_debounce.borrow().as_ref() {
            prev.store(true, Ordering::Relaxed);
        }

        let flag = Arc::new(AtomicBool::new(false));
        *imp.search_debounce.borrow_mut() = Some(flag.clone());

        let win = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if flag.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }
            win.run_search(query.clone());
            glib::ControlFlow::Break
        });
    }

    fn run_search(&self, query: String) {
        let imp = self.imp();
        *imp.last_query.borrow_mut() = query.clone();

        let active_filter = imp.filter_state.borrow().clone();

        let cancel = Arc::new(AtomicBool::new(false));
        { if let Some(prev) = imp.search_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }}
        *imp.search_cancel.borrow_mut() = Some(cancel);

        let list = imp.commit_list.clone();
        list.remove_all();

        let all = imp.all_commits.borrow().clone();
        let selected_year = imp.selected_year.get();

        if !query.is_empty() {
            *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
            imp.timeline_stack.set_visible_child_name("commits");
            imp.timeline_back_button.set_visible(true);
            imp.timeline_header_title.set_subtitle("");
        }

        let filtered: Vec<CommitInfo> = if query.is_empty() && active_filter.is_empty() {
            if selected_year != 0 {
                all.into_iter()
                    .filter(|c| {
                        glib::DateTime::from_unix_local(c.timestamp)
                            .map(|d| d.year() == selected_year)
                            .unwrap_or(false)
                    })
                    .collect()
            } else {
                all
            }
        } else {
            let q = query.to_lowercase();
            all.into_iter()
                .filter(|c| {
                    let year_ok = selected_year == 0 || {
                        glib::DateTime::from_unix_local(c.timestamp)
                            .map(|d| d.year() == selected_year)
                            .unwrap_or(false)
                    };
                    if !year_ok { return false; }

                    if !active_filter.matches(c) { return false; }

                    if q.is_empty() { return true; }

                    let short_hash = &c.hash[..7.min(c.hash.len())];
                    let text_match =
                        c.summary.to_lowercase().contains(&q)
                        || c.hash.to_lowercase().contains(&q)
                        || short_hash.to_lowercase().starts_with(&q)
                        || c.author.to_lowercase().contains(&q);

                    let date_match = matches_calendar(c.timestamp, &q);

                    text_match || date_match
                })
                .collect()
        };

        commit_controller::populate_commit_list(&list, &filtered);
    }

    // ── Filter popover wiring ─────────────────────────────────────────────────

    fn setup_filter_popover(&self) {
        let popover = SearchFilterPopover::new();

        let btn = gtk::ToggleButton::builder()
            .icon_name("funnel-symbolic")
            .tooltip_text(gettext("Filters"))
            .css_classes(vec!["flat".to_string()])
            .build();

        popover.set_parent(&btn);

        {
            let pop = popover.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() { pop.popup(); } else { pop.popdown(); }
            });
        }

        {
            let b = btn.clone();
            popover.connect_closed(move |_| { b.set_active(false); });
        }

        {
            let win = self.clone();
            popover.connect_local("filters-changed", false, move |_| {
                let pop = win.imp().filter_popover.borrow();
                if let Some(ref p) = *pop {
                    *win.imp().filter_state.borrow_mut() = p.current_filter();
                }
                drop(pop);
                let q = win.imp().last_query.borrow().clone();
                win.run_search(q);
                None
            });
        }

        if let Some(parent) = self.imp().commit_search_entry
            .parent()
            .and_then(|w| w.downcast::<gtk::Box>().ok())
        {
            parent.append(&btn);
        }

        *self.imp().filter_button.borrow_mut()  = Some(btn);
        *self.imp().filter_popover.borrow_mut() = Some(popover);
    }

    // ── Error display ─────────────────────────────────────────────────────────

    fn show_error(&self, message: &str) {
        eprintln!("[TemporalExplorer] {message}");
        let dialog = adw::AlertDialog::new(Some(&gettext("Error")), Some(message));
        AlertDialogExt::add_response(&dialog, "ok", &gettext("OK"));
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── Timestamp formatter ───────────────────────────────────────────────────

    fn format_timestamp(ts: i64) -> String {
        glib::DateTime::from_unix_local(ts)
            .and_then(|d| d.format("%Y-%m-%d %H:%M"))
            .map(|s| s.to_string())
            .unwrap_or_else(|_| ts.to_string())
    }
}

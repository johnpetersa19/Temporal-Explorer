/* tabs.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Tab lifecycle: state capture/restore, titles, overview and tab moves.

use super::*;
use super::util::*;

impl TemporalExplorerWindow {
    pub(super) fn tab_key(page: &adw::TabPage) -> usize {
        page.as_ptr() as usize
    }

    pub(super) fn capture_tab_state(&self) -> TabState {
        let imp = self.imp();
        TabState {
            hash: imp.current_hash.borrow().clone(),
            oid: imp.current_oid.get(),
            dir: imp.current_dir.borrow().clone(),
            nodes: imp.current_nodes.borrow().clone(),
            history_back: imp.history_back.borrow().clone(),
            history_forward: imp.history_forward.borrow().clone(),
            commit_nav_back: imp.commit_nav_back.borrow().clone(),
            commit_nav_forward: imp.commit_nav_forward.borrow().clone(),
        }
    }

    pub(super) fn save_active_tab_state(&self) {
        if self.imp().switching_tab.get() {
            return;
        }
        if let Some(page) = self.imp().tab_view.selected_page() {
            self.imp()
                .tab_states
                .borrow_mut()
                .insert(Self::tab_key(&page), self.capture_tab_state());
            self.update_tab_title(&page);
        }
    }

    pub(super) fn restore_selected_tab_state(&self) {
        let imp = self.imp();
        let Some(page) = imp.tab_view.selected_page() else {
            return;
        };
        let Some(state) = imp.tab_states.borrow().get(&Self::tab_key(&page)).cloned() else {
            return;
        };

        imp.switching_tab.set(true);
        *imp.current_hash.borrow_mut() = state.hash.clone();
        imp.current_oid.set(state.oid);
        *imp.current_dir.borrow_mut() = state.dir.clone();
        *imp.current_nodes.borrow_mut() = state.nodes;
        *imp.history_back.borrow_mut() = state.history_back;
        *imp.history_forward.borrow_mut() = state.history_forward;
        *imp.commit_nav_back.borrow_mut() = state.commit_nav_back;
        *imp.commit_nav_forward.borrow_mut() = state.commit_nav_forward;
        imp.switching_tab.set(false);

        self.sync_commit_info_for_hash(state.hash.as_deref());
        self.update_navigation_buttons();
        if state.hash.is_some() {
            self.navigate_to_dir(state.dir);
        } else {
            self.show_empty_state();
        }
    }

    pub(super) fn sync_commit_info_for_hash(&self, hash: Option<&str>) {
        let imp = self.imp();
        let commit = hash.and_then(|hash| {
            let commits = imp.all_commits.borrow();
            let index = imp.commit_index.borrow();
            index
                .get(hash)
                .and_then(|position| commits.get(*position))
                .map(|commit| {
                    (
                        commit.hash.clone(),
                        commit.summary.clone(),
                        commit.timestamp,
                    )
                })
        });
        if let Some((hash, summary, timestamp)) = commit {
            imp.commit_hash_label.set_label(display_hash(&hash));
            imp.commit_message_label.set_label(&summary);
            imp.commit_date_label
                .set_label(&Self::format_timestamp(timestamp));
            imp.commit_info_bar.set_revealed(true);
        } else {
            imp.commit_info_bar.set_revealed(false);
        }
    }

    pub(super) fn update_tab_title(&self, page: &adw::TabPage) {
        let imp = self.imp();
        let dir = imp.current_dir.borrow();
        let title = dir
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let repo = imp.repo_name.borrow();
                if repo.is_empty() {
                    gettext("New Tab")
                } else {
                    repo.clone()
                }
            });
        page.set_title(&title);
        if imp.tab_view.selected_page().as_ref() == Some(page) {
            self.set_title(Some(&title));
        }
        let tooltip = if dir.as_os_str().is_empty() {
            title.clone()
        } else {
            dir.to_string_lossy().into_owned()
        };
        page.set_tooltip(&tooltip);
    }

    pub(super) fn setup_tabs(&self) {
        let imp = self.imp();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .vexpand(true)
            .hexpand(true)
            .build();
        let page = imp.tab_view.append(&content);
        page.set_title(&gettext("New Tab"));
        imp.tab_states
            .borrow_mut()
            .insert(Self::tab_key(&page), TabState::default());

        let win = self.clone();
        imp.tab_view.connect_selected_page_notify(move |view| {
            if let Some(page) = view.selected_page() {
                let title = page.title();
                let display_title = if title.is_empty() {
                    gettext("Temporal Explorer")
                } else {
                    title.to_string()
                };
                win.set_title(Some(display_title.as_str()));
            }
            win.restore_selected_tab_state();
        });

        let win = self.clone();
        imp.tab_view.connect_close_page(move |view, page| {
            if let Some(state) = win
                .imp()
                .tab_states
                .borrow()
                .get(&Self::tab_key(page))
                .cloned()
            {
                let mut closed_tabs = win.imp().closed_tabs.borrow_mut();
                closed_tabs.push((state, page.title().to_string()));
                if closed_tabs.len() > 10 {
                    closed_tabs.remove(0);
                }
            }
            if view.n_pages() <= 1 {
                win.close();
            } else {
                view.close_page_finish(page, true);
            }
            glib::Propagation::Stop
        });

        let win = self.clone();
        imp.tab_view.connect_page_detached(move |_, page, _| {
            win.imp()
                .tab_states
                .borrow_mut()
                .remove(&Self::tab_key(page));
        });

        let win = self.clone();
        imp.tab_view.connect_setup_menu(move |view, page| {
            let n_pages = view.n_pages();
            let pos = page.map(|p| view.page_position(p)).unwrap_or(-1);
            let set_enabled = |name: &str, enabled: bool| {
                if let Some(action) = win
                    .lookup_action(name)
                    .and_downcast::<gio::SimpleAction>()
                {
                    action.set_enabled(enabled);
                }
            };
            set_enabled("move-tab-left", pos > 0);
            set_enabled("move-tab-right", pos >= 0 && pos + 1 < n_pages);
            set_enabled("close-other-tabs", n_pages > 1);
            set_enabled("move-tab-to-new-window", n_pages > 1);
            set_enabled(
                "restore-closed-tab",
                !win.imp().closed_tabs.borrow().is_empty(),
            );
        });

        let win = self.clone();
        imp.tab_view.connect_create_window(move |_| {
            let application = win.application()?;
            let new_window = TemporalExplorerWindow::new(&application);
            if let Some(repo_path) = win.imp().repo_path.borrow().clone() {
                new_window.load_repository(repo_path);
            }
            new_window.present();
            Some(new_window.imp().tab_view.get())
        });

        self.show_empty_state();
    }

    pub(super) fn new_tab(&self) {
        self.new_tab_page();
    }

    pub(super) fn new_tab_page(&self) -> adw::TabPage {
        self.save_active_tab_state();
        let state = self.capture_tab_state();
        self.create_tab_with_state(state, gettext("New Tab"))
    }

    pub(super) fn create_tab_with_state(&self, state: TabState, title: String) -> adw::TabPage {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .vexpand(true)
            .hexpand(true)
            .build();
        let page = self.imp().tab_view.append(&content);
        self.imp()
            .tab_states
            .borrow_mut()
            .insert(Self::tab_key(&page), state.clone());
        page.set_title(&title);
        if state.hash.is_some() {
            self.update_tab_title(&page);
        }
        self.imp().tab_view.set_selected_page(&page);
        self.restore_selected_tab_state();
        page
    }

    pub(super) fn close_current_tab(&self) {
        let imp = self.imp();
        let Some(page) = imp.tab_view.selected_page() else {
            return;
        };
        if imp.tab_view.n_pages() <= 1 {
            self.close();
        } else {
            imp.tab_view.close_page(&page);
        }
    }

    pub(super) fn open_tab_overview(&self) {
        self.imp().tab_overview.set_open(true);
    }

    pub(super) fn move_tab_to_new_window(&self) {
        let Some(application) = self.application() else {
            self.show_error(&gettext("Could not open a new window"));
            return;
        };
        let Some(page) = self.imp().tab_view.selected_page() else {
            return;
        };

        self.save_active_tab_state();
        let key = Self::tab_key(&page);
        let state = self
            .imp()
            .tab_states
            .borrow()
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let title = page.title().to_string();
        let new_window = TemporalExplorerWindow::new(&application);
        let repo_path = self.imp().repo_path.borrow().clone();

        if let Some(repo_path) = repo_path {
            *new_window.imp().pending_tab_state.borrow_mut() = Some((state, title));
            new_window.load_repository(repo_path);
        } else if let Some(new_page) = new_window.imp().tab_view.selected_page() {
            new_window
                .imp()
                .tab_states
                .borrow_mut()
                .insert(Self::tab_key(&new_page), state);
            new_page.set_title(&title);
            new_window.restore_selected_tab_state();
        }

        new_window.present();
        self.imp().tab_states.borrow_mut().remove(&key);
        self.imp().tab_view.close_page(&page);
    }

    pub(super) fn restore_closed_tab(&self) {
        let Some((state, title)) = self.imp().closed_tabs.borrow_mut().pop() else {
            self.show_toast(&gettext("No closed tab to restore"));
            return;
        };
        self.save_active_tab_state();
        self.create_tab_with_state(state, title);
    }

    pub(super) fn close_other_tabs(&self) {
        let imp = self.imp();
        let Some(page) = imp.tab_view.selected_page() else {
            return;
        };
        self.save_active_tab_state();
        imp.tab_view.close_other_pages(&page);
        let key = Self::tab_key(&page);
        let selected_state = imp.tab_states.borrow().get(&key).cloned();
        imp.tab_states.borrow_mut().clear();
        if let Some(state) = selected_state {
            imp.tab_states.borrow_mut().insert(key, state);
        }
        self.restore_selected_tab_state();
    }

    pub(super) fn active_tab_content(&self) -> Option<gtk::Box> {
        self.imp()
            .tab_view
            .selected_page()
            .and_then(|page| page.child().downcast::<gtk::Box>().ok())
    }

    pub fn replace_right_panel(&self, widget: gtk::Widget) {
        self.close_active_context_popover();
        if let Some(content) = self.active_tab_content() {
            clear_box(&content);
            content.append(&widget);
        }
    }

    pub fn show_empty_state(&self) {
        self.close_active_context_popover();
        if let Some(content) = self.active_tab_content() {
            clear_box(&content);
            let empty = adw::StatusPage::builder()
                .icon_name("folder-symbolic")
                .title(gettext("Temporal-Explorer"))
                .description(gettext(
                    "Open a Git repository and select a commit from the timeline to browse its file tree.",
                ))
                .vexpand(true)
                .hexpand(true)
                .build();
            content.append(&empty);
        }
        self.save_active_tab_state();
    }

    pub(super) fn new_window(&self) {
        let Some(application) = self.application() else {
            self.show_error(&gettext("Could not open a new window"));
            return;
        };
        let new_win = TemporalExplorerWindow::new(&application);
        if let Some(repo_path) = self.imp().repo_path.borrow().clone() {
            new_win.load_repository(repo_path);
        }
        new_win.present();
    }

    pub(super) fn move_tab_left(&self) {
        let imp = self.imp();
        let Some(page) = imp.tab_view.selected_page() else {
            return;
        };
        let pos = imp.tab_view.page_position(&page);
        if pos > 0 {
            imp.tab_view.reorder_page(&page, pos - 1);
        }
    }

    pub(super) fn move_tab_right(&self) {
        let imp = self.imp();
        let Some(page) = imp.tab_view.selected_page() else {
            return;
        };
        let pos = imp.tab_view.page_position(&page);
        if pos + 1 < imp.tab_view.n_pages() {
            imp.tab_view.reorder_page(&page, pos + 1);
        }
    }

}

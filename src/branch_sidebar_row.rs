/* branch_sidebar_row.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Sidebar row for a branch. Emits three signals:
 *   - branch-checked-out(name: &str)
 *   - branch-deleted(name: &str)
 *   - branch-push-requested(name: &str)
 *
 * Action buttons are revealed via a Revealer on hover / focus-within
 * using motion and focus controllers wired in setup().
 */

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::RefCell;
use std::sync::OnceLock;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/branch-sidebar-row.ui")]
    pub struct BranchSidebarRow {
        #[template_child] pub branch_icon:        TemplateChild<gtk::Image>,
        #[template_child] pub branch_name_label:  TemplateChild<gtk::Label>,
        #[template_child] pub head_badge:         TemplateChild<gtk::Label>,
        #[template_child] pub actions_revealer:   TemplateChild<gtk::Revealer>,
        #[template_child] pub checkout_button:    TemplateChild<gtk::Button>,
        #[template_child] pub push_button:        TemplateChild<gtk::Button>,
        #[template_child] pub rename_button:      TemplateChild<gtk::Button>,
        #[template_child] pub delete_button:      TemplateChild<gtk::Button>,

        pub branch_name: RefCell<String>,
        pub is_remote:   RefCell<bool>,
        pub is_head:     RefCell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BranchSidebarRow {
        const NAME: &'static str = "BranchSidebarRow";
        type Type = super::BranchSidebarRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BranchSidebarRow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_hover_controllers();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![
                glib::subclass::Signal::builder("branch-checked-out")
                    .param_types([String::static_type()])
                    .build(),
                glib::subclass::Signal::builder("branch-deleted")
                    .param_types([String::static_type()])
                    .build(),
                glib::subclass::Signal::builder("branch-push-requested")
                    .param_types([String::static_type()])
                    .build(),
            ])
        }
    }

    impl WidgetImpl     for BranchSidebarRow {}
    impl ListBoxRowImpl for BranchSidebarRow {}

    #[gtk::template_callbacks]
    impl BranchSidebarRow {
        #[template_callback]
        fn on_checkout_clicked(&self) {
            let name = self.branch_name.borrow().clone();
            self.obj().emit_by_name::<()>("branch-checked-out", &[&name.to_value()]);
        }

        #[template_callback]
        fn on_push_clicked(&self) {
            let name = self.branch_name.borrow().clone();
            self.obj().emit_by_name::<()>("branch-push-requested", &[&name.to_value()]);
        }

        #[template_callback]
        fn on_rename_clicked(&self) {
            // Opens an inline entry via window.rs — emits no signal here;
            // the caller connects to this button directly after bind_branch().
        }

        #[template_callback]
        fn on_delete_clicked(&self) {
            let name = self.branch_name.borrow().clone();
            self.obj().emit_by_name::<()>("branch-deleted", &[&name.to_value()]);
        }
    }
}

// ── Public wrapper ───────────────────────────────────────────────────

glib::wrapper! {
    pub struct BranchSidebarRow(ObjectSubclass<imp::BranchSidebarRow>)
        @extends gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable;
}

impl Default for BranchSidebarRow {
    fn default() -> Self { Self::new() }
}

impl BranchSidebarRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Populate the row from branch metadata.
    pub fn bind_branch(&self, name: &str, is_remote: bool, is_head: bool) {
        let imp = self.imp();
        *imp.branch_name.borrow_mut() = name.to_string();
        *imp.is_remote.borrow_mut()   = is_remote;
        *imp.is_head.borrow_mut()     = is_head;

        imp.branch_name_label.set_label(name);
        imp.branch_icon.set_icon_name(Some(
            if is_remote { "network-server-symbolic" } else { "branch-symbolic" }
        ));
        imp.head_badge.set_visible(is_head);
        // Remote branches cannot be pushed from here
        imp.push_button.set_visible(!is_remote);
        // HEAD branch cannot be deleted
        imp.delete_button.set_sensitive(!is_head);
    }

    pub fn connect_branch_checked_out<F>(&self, f: F) -> glib::SignalHandlerId
    where F: Fn(&Self, &str) + 'static {
        self.connect_local("branch-checked-out", false, move |v| {
            let row = v[0].get::<BranchSidebarRow>().unwrap();
            let name = v[1].get::<String>().unwrap();
            f(&row, &name); None
        })
    }

    pub fn connect_branch_deleted<F>(&self, f: F) -> glib::SignalHandlerId
    where F: Fn(&Self, &str) + 'static {
        self.connect_local("branch-deleted", false, move |v| {
            let row = v[0].get::<BranchSidebarRow>().unwrap();
            let name = v[1].get::<String>().unwrap();
            f(&row, &name); None
        })
    }

    pub fn connect_branch_push_requested<F>(&self, f: F) -> glib::SignalHandlerId
    where F: Fn(&Self, &str) + 'static {
        self.connect_local("branch-push-requested", false, move |v| {
            let row = v[0].get::<BranchSidebarRow>().unwrap();
            let name = v[1].get::<String>().unwrap();
            f(&row, &name); None
        })
    }

    pub fn rename_button(&self) -> gtk::Button {
        self.imp().rename_button.get()
    }

    // ── Hover / focus controllers ────────────────────────────────────────

    fn setup_hover_controllers(&self) {
        // Reveal action buttons on mouse enter, hide on leave
        let motion = gtk::EventControllerMotion::new();
        let row_weak = self.downgrade();
        motion.connect_enter(move |_, _, _| {
            if let Some(row) = row_weak.upgrade() {
                row.imp().actions_revealer.set_reveal_child(true);
            }
        });
        let row_weak = self.downgrade();
        motion.connect_leave(move |_| {
            if let Some(row) = row_weak.upgrade() {
                // Don't hide if a button inside has keyboard focus
                if !row.imp().actions_revealer.is_focus() {
                    row.imp().actions_revealer.set_reveal_child(false);
                }
            }
        });
        self.add_controller(motion);

        // Also reveal when the row itself gains keyboard focus
        let focus = gtk::EventControllerFocus::new();
        let row_weak = self.downgrade();
        focus.connect_enter(move |_| {
            if let Some(row) = row_weak.upgrade() {
                row.imp().actions_revealer.set_reveal_child(true);
            }
        });
        let row_weak = self.downgrade();
        focus.connect_leave(move |_| {
            if let Some(row) = row_weak.upgrade() {
                row.imp().actions_revealer.set_reveal_child(false);
            }
        });
        self.add_controller(focus);
    }
}

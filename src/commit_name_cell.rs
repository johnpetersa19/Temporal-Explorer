/* commit_name_cell.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Rich timeline cell. Displays:
 *  - Author avatar (coloured circle with first letter)
 *  - Commit summary with ellipsis
 *  - Inline SHA copy button (copies full SHA to clipboard)
 *  - Diff-stat +N / −N emblems
 *  - SHA short label + author name + relative date
 *  - Branch / tag badge chips (FlowBox, hidden when empty)
 */

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use adw::prelude::*;
use std::cell::RefCell;

// ── Palette for avatar background based on author email hash ────────────────
const AVATAR_COLORS: &[&str] = &[
    "#3584e4", "#33d17a", "#f6d32d", "#ff7800",
    "#e01b24", "#9141ac", "#2190a4", "#c64600",
];

fn avatar_color(email: &str) -> &'static str {
    let hash: usize = email.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    AVATAR_COLORS[hash % AVATAR_COLORS.len()]
}

// ── Data model passed to bind_commit ─────────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct CommitCellData {
    pub sha:        String,   // full 40-char SHA
    pub summary:    String,
    pub author:     String,
    pub email:      String,
    pub date_rel:   String,   // e.g. "2 days ago"
    pub additions:  i32,
    pub deletions:  i32,
    /// (name, is_tag) pairs — branches first, tags second
    pub refs:       Vec<(String, bool)>,
}

// ── GObject subclass ───────────────────────────────────────────────────
mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/commit-name-cell.ui")]
    pub struct CommitNameCell {
        #[template_child] pub summary_label:    TemplateChild<gtk::Label>,
        #[template_child] pub sha_label:        TemplateChild<gtk::Label>,
        #[template_child] pub sha_copy_button:  TemplateChild<gtk::Button>,
        #[template_child] pub author_label:     TemplateChild<gtk::Label>,
        #[template_child] pub date_label:       TemplateChild<gtk::Label>,
        #[template_child] pub author_avatar:    TemplateChild<gtk::Label>,
        #[template_child] pub additions_label:  TemplateChild<gtk::Label>,
        #[template_child] pub deletions_label:  TemplateChild<gtk::Label>,
        #[template_child] pub badges_box:       TemplateChild<gtk::FlowBox>,

        pub full_sha: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CommitNameCell {
        const NAME: &'static str = "CommitNameCell";
        type Type = super::CommitNameCell;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl        for CommitNameCell {}
    impl WidgetImpl        for CommitNameCell {}
    impl ListBoxRowImpl    for CommitNameCell {}

    #[gtk::template_callbacks]
    impl CommitNameCell {
        #[template_callback]
        fn on_sha_copy_clicked(&self) {
            let sha = self.full_sha.borrow().clone();
            if let Some(display) = self.obj().display().downcast_ref::<gtk::gdk::Display>() {
                display.clipboard().set_text(&sha);
            }
            // Show a brief toast via the parent window if available
            if let Some(win) = self.obj().root().and_downcast::<adw::ApplicationWindow>() {
                if let Some(toast_overlay) = win
                    .content()
                    .and_downcast::<adw::ToastOverlay>()
                {
                    let toast = adw::Toast::new(&format!("Copied {}", &sha[..8]));
                    toast.set_timeout(2);
                    toast_overlay.add_toast(toast);
                }
            }
        }
    }
}

// ── Public wrapper ───────────────────────────────────────────────────

glib::wrapper! {
    pub struct CommitNameCell(ObjectSubclass<imp::CommitNameCell>)
        @extends gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable;
}

impl Default for CommitNameCell {
    fn default() -> Self { Self::new() }
}

impl CommitNameCell {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Populate all fields from a `CommitCellData`.
    pub fn bind_commit(&self, data: &CommitCellData) {
        let imp = self.imp();

        // Store full SHA for clipboard
        *imp.full_sha.borrow_mut() = data.sha.clone();

        // Summary
        imp.summary_label.set_label(&data.summary);

        // Short SHA (7 chars)
        let short = if data.sha.len() >= 7 { &data.sha[..7] } else { &data.sha };
        imp.sha_label.set_label(short);

        // Author & date
        imp.author_label.set_label(&data.author);
        imp.date_label.set_label(&data.date_rel);

        // Avatar: first letter of author name, coloured background via CSS
        let initial = data.author.chars().next().unwrap_or('?').to_uppercase().next().unwrap_or('?');
        imp.author_avatar.set_label(&initial.to_string());
        // Inline style for avatar colour (safe: only hex colours from AVATAR_COLORS)
        let color = avatar_color(&data.email);
        imp.author_avatar.set_css_classes(&["commit-avatar"]);
        // Use a CSS custom property trick via the widget name as a selector anchor
        imp.author_avatar.set_widget_name(&format!("avatar-{}", &data.sha[..7]));

        // Diff stat
        if data.additions > 0 || data.deletions > 0 {
            imp.additions_label.set_label(&format!("+{}", data.additions));
            imp.deletions_label.set_label(&format!("−{}", data.deletions));
            imp.additions_label.set_visible(true);
            imp.deletions_label.set_visible(true);
        } else {
            imp.additions_label.set_visible(false);
            imp.deletions_label.set_visible(false);
        }

        // Branch / tag badges
        let badges = imp.badges_box.get();
        // Remove old chips
        while let Some(child) = badges.first_child() {
            badges.remove(&child);
        }
        if data.refs.is_empty() {
            badges.set_visible(false);
        } else {
            for (name, is_tag) in &data.refs {
                let chip = gtk::Label::builder()
                    .label(name)
                    .build();
                if *is_tag {
                    chip.add_css_class("tag-badge");
                    chip.add_css_class("badge");
                } else {
                    chip.add_css_class("branch-badge");
                    chip.add_css_class("badge");
                }
                let item = gtk::FlowBoxChild::builder().child(&chip).build();
                badges.append(&item);
            }
            badges.set_visible(true);
        }

        // Avatar background colour applied via provider
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!(
            "#avatar-{short} {{ background-color: {color}; color: white; }}"
        ));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }
}

/* favorites_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use std::cell::RefCell;
use std::path::PathBuf;

pub type OnOpenFavorite = Box<dyn Fn(PathBuf, String, PathBuf) + 'static>;
pub type OnRemoveFavorite = Box<dyn Fn(String) + 'static>;

#[derive(Debug, Clone)]
pub struct FavoriteEntry {
    pub raw_id: String,
    pub repo_path: PathBuf,
    pub hash: String,
    pub item_path: PathBuf,
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/favorites-dialog.ui")]
    pub struct FavoritesDialog {
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub favorites_group: TemplateChild<adw::PreferencesGroup>,

        pub rows: RefCell<Vec<adw::ActionRow>>,
        pub on_open: RefCell<Option<OnOpenFavorite>>,
        pub on_remove: RefCell<Option<OnRemoveFavorite>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FavoritesDialog {
        const NAME: &'static str = "FavoritesDialog";
        type Type = super::FavoritesDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FavoritesDialog {}
    impl WidgetImpl for FavoritesDialog {}
    impl AdwDialogImpl for FavoritesDialog {}
}

glib::wrapper! {
    pub struct FavoritesDialog(ObjectSubclass<imp::FavoritesDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl Default for FavoritesDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl FavoritesDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_callbacks(&self, on_open: OnOpenFavorite, on_remove: OnRemoveFavorite) {
        *self.imp().on_open.borrow_mut() = Some(on_open);
        *self.imp().on_remove.borrow_mut() = Some(on_remove);
    }

    pub fn set_entries(&self, entries: Vec<FavoriteEntry>) {
        let imp = self.imp();
        for row in imp.rows.borrow_mut().drain(..) {
            imp.favorites_group.remove(&row);
        }

        if entries.is_empty() {
            imp.content_stack.set_visible_child_name("empty");
            return;
        }

        imp.content_stack.set_visible_child_name("list");

        for entry in entries {
            let repo_name = entry
                .repo_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_else(|| entry.repo_path.to_str().unwrap_or_default())
                .to_string();
            let title = entry
                .item_path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| repo_name.clone());
            let short = &entry.hash[..8.min(entry.hash.len())];
            let path_display = if entry.item_path.as_os_str().is_empty() {
                "/".to_string()
            } else {
                entry.item_path.display().to_string()
            };
            let subtitle = format!("{repo_name} · {short} · {path_display}");

            let row = adw::ActionRow::builder()
                .title(&title)
                .subtitle(&subtitle)
                .activatable(true)
                .use_markup(false)
                .build();

            let star_icon = gtk::Image::from_icon_name("starred-symbolic");
            row.add_prefix(&star_icon);

            let remove_btn = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(gettext("Remove from Favorites"))
                .valign(gtk::Align::Center)
                .build();
            remove_btn.add_css_class("flat");
            row.add_suffix(&remove_btn);

            let chevron = gtk::Image::from_icon_name("go-next-symbolic");
            chevron.add_css_class("dim-label");
            row.add_suffix(&chevron);

            let dialog = self.clone();
            let repo = entry.repo_path.clone();
            let hash = entry.hash.clone();
            let path = entry.item_path.clone();
            row.connect_activated(move |_| {
                if let Some(cb) = dialog.imp().on_open.borrow().as_ref() {
                    cb(repo.clone(), hash.clone(), path.clone());
                }
                dialog.close();
            });

            let dialog = self.clone();
            let row_ref = row.clone();
            let raw_id = entry.raw_id.clone();
            remove_btn.connect_clicked(move |_| {
                if let Some(cb) = dialog.imp().on_remove.borrow().as_ref() {
                    cb(raw_id.clone());
                }
                dialog.imp().favorites_group.remove(&row_ref);
                dialog
                    .imp()
                    .rows
                    .borrow_mut()
                    .retain(|r| r != &row_ref);
                if dialog.imp().rows.borrow().is_empty() {
                    dialog.imp().content_stack.set_visible_child_name("empty");
                }
            });

            imp.favorites_group.add(&row);
            imp.rows.borrow_mut().push(row);
        }
    }
}

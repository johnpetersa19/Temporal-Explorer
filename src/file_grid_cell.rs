/* file_grid_cell.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/file-grid-cell.ui")]
    pub struct FileGridCell {
        #[template_child]
        pub grid_cell_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub grid_cell_name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub captions_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub emblems_box: TemplateChild<gtk::Box>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileGridCell {
        const NAME: &'static str = "FileGridCell";
        type Type = super::FileGridCell;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FileGridCell {}
    impl WidgetImpl for FileGridCell {}
    impl BoxImpl for FileGridCell {}
}

glib::wrapper! {
    pub struct FileGridCell(ObjectSubclass<imp::FileGridCell>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for FileGridCell {
    fn default() -> Self {
        Self::new()
    }
}

impl FileGridCell {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_icon_name(&self, icon_name: &str) {
        self.imp().grid_cell_icon.set_icon_name(Some(icon_name));
    }

    pub fn set_icon_size(&self, size: i32) {
        self.imp().grid_cell_icon.set_pixel_size(size);
    }

    pub fn set_name(&self, name: &str) {
        let label = self.imp().grid_cell_name_label.get();
        label.set_label(name);
        label.set_tooltip_text(Some(name));
    }

    pub fn set_label_width_chars(&self, chars: i32) {
        self.imp().grid_cell_name_label.set_max_width_chars(chars);
    }

    pub fn set_cell_width(&self, width: i32) {
        self.set_width_request(width);
    }

    pub fn clear_captions(&self) {
        clear_box(&self.imp().captions_box);
    }

    pub fn add_caption(&self, caption: &str) {
        let label = gtk::Label::builder()
            .label(caption)
            .halign(gtk::Align::Center)
            .justify(gtk::Justification::Center)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(self.imp().grid_cell_name_label.max_width_chars())
            .build();

        label.add_css_class("caption");
        label.add_css_class("dim-label");
        self.imp().captions_box.append(&label);
    }

    pub fn clear_emblems(&self) {
        clear_box(&self.imp().emblems_box);
    }

    pub fn add_emblem(&self, icon_name: &str) {
        let emblem = gtk::Image::from_icon_name(icon_name);
        emblem.set_pixel_size(16);
        emblem.add_css_class("nautilus-emblem");
        self.imp().emblems_box.append(&emblem);
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

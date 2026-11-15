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
    #[template(resource = "/io/github/TemporalExplorer/file-grid-cell.ui")]
    pub struct FileGridCell {
        #[template_child]
        pub grid_cell_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub grid_cell_name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub captions_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub emblems_box: TemplateChild<gtk::Box>,

        /// Fixed width of the cell, in pixels.
        ///
        /// Mirrors the Nautilus grid cell, which reports the very same
        /// minimum and natural width so that a long label can never grow
        /// a column past the visible area.
        pub cell_width: std::cell::Cell<i32>,
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
    impl WidgetImpl for FileGridCell {
        /// Reports a fixed width for the cell, as the Nautilus does.
        ///
        /// The Nautilus grid cell measures itself as
        /// `18 + icon_size + 18` pixels and reports that value as both the
        /// minimum and the natural width.  Because the two are equal, the
        /// `GtkGridView` never lays out one column more than it can show
        /// and the last column is never clipped.  Here the same trick is
        /// used: the name label wraps (and ellipsizes) inside the cell
        /// instead of widening it.
        fn measure(
            &self,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let width = self.cell_width.get();

            if orientation == gtk::Orientation::Horizontal && width > 0 {
                return (width, width, -1, -1);
            }

            // The vertical measurement is constrained to the cell width, so
            // that the label is measured with the space it will actually
            // get and the cell reserves the right amount of height.
            let for_size = if width > 0 { width } else { for_size };
            self.parent_measure(orientation, for_size)
        }
    }
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

    pub fn set_gicon(&self, icon: &impl IsA<gtk::gio::Icon>) {
        self.imp().grid_cell_icon.set_from_gicon(icon);
    }

    pub fn set_paintable(&self, paintable: Option<&impl IsA<gtk::gdk::Paintable>>) {
        self.imp().grid_cell_icon.set_paintable(paintable);
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

    /// Sets the width of a grid cell.
    ///
    /// The cell reports this width as both its minimum and natural width
    /// (see the `measure` override), so the `GtkGridView` never places one
    /// column more than it can show - the last column is never clipped.
    pub fn set_cell_width(&self, width: i32) {
        self.imp().cell_width.set(width);
        self.set_width_request(width);

        // Discounts the cell side margins (6 + 6) from the label width.
        let label_width = (width - 12).max(40);
        self.imp().grid_cell_name_label.set_width_request(label_width);
    }

    pub fn clear_captions(&self) {
        clear_box(&self.imp().captions_box);
    }

    pub fn add_caption(&self, caption: &str) {
        let label = gtk::Label::builder()
            .label(caption)
            .halign(gtk::Align::Fill)
            .justify(gtk::Justification::Center)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .lines(1)
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

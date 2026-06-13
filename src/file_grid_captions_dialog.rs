/* file_grid_captions_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Dialog to toggle caption visibility in the file grid view.
 * Emits `captions-changed` with a `CaptionFlags` bitmask when Apply is clicked.
 */

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use adw::prelude::*;
use std::sync::OnceLock;

// ── Caption flags bitmask ─────────────────────────────────────────────────
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct CaptionFlags: u32 {
        const STATUS    = 0b0001;
        const EXTENSION = 0b0010;
        const SIZE      = 0b0100;
        const DATE      = 0b1000;
    }
}

// ── GObject subclass ──────────────────────────────────────────────────
mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/file-grid-captions-dialog.ui")]
    pub struct FileGridCaptionsDialog {
        #[template_child] pub caption_status_row:    TemplateChild<adw::SwitchRow>,
        #[template_child] pub caption_extension_row: TemplateChild<adw::SwitchRow>,
        #[template_child] pub caption_size_row:      TemplateChild<adw::SwitchRow>,
        #[template_child] pub caption_date_row:      TemplateChild<adw::SwitchRow>,
        #[template_child] pub apply_button:          TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileGridCaptionsDialog {
        const NAME: &'static str = "FileGridCaptionsDialog";
        type Type = super::FileGridCaptionsDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FileGridCaptionsDialog {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![
                glib::subclass::Signal::builder("captions-changed")
                    .param_types([u32::static_type()])
                    .build(),
            ])
        }
    }

    impl WidgetImpl    for FileGridCaptionsDialog {}
    impl AdwDialogImpl for FileGridCaptionsDialog {}

    #[gtk::template_callbacks]
    impl FileGridCaptionsDialog {
        #[template_callback]
        fn on_apply_clicked(&self) {
            self.obj().emit_captions_changed();
            self.obj().close();
        }
    }
}

// ── Public wrapper ───────────────────────────────────────────────────

glib::wrapper! {
    pub struct FileGridCaptionsDialog(ObjectSubclass<imp::FileGridCaptionsDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for FileGridCaptionsDialog {
    fn default() -> Self { Self::new() }
}

impl FileGridCaptionsDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Pre-fill toggles from stored settings before presenting.
    pub fn set_flags(&self, flags: CaptionFlags) {
        let imp = self.imp();
        imp.caption_status_row.set_active(flags.contains(CaptionFlags::STATUS));
        imp.caption_extension_row.set_active(flags.contains(CaptionFlags::EXTENSION));
        imp.caption_size_row.set_active(flags.contains(CaptionFlags::SIZE));
        imp.caption_date_row.set_active(flags.contains(CaptionFlags::DATE));
    }

    /// Read current flags from the toggle state.
    pub fn current_flags(&self) -> CaptionFlags {
        let imp = self.imp();
        let mut f = CaptionFlags::empty();
        if imp.caption_status_row.is_active()    { f |= CaptionFlags::STATUS; }
        if imp.caption_extension_row.is_active() { f |= CaptionFlags::EXTENSION; }
        if imp.caption_size_row.is_active()      { f |= CaptionFlags::SIZE; }
        if imp.caption_date_row.is_active()      { f |= CaptionFlags::DATE; }
        f
    }

    pub fn connect_captions_changed<F>(&self, f: F) -> glib::SignalHandlerId
    where F: Fn(&Self, CaptionFlags) + 'static {
        self.connect_local("captions-changed", false, move |v| {
            let dlg   = v[0].get::<FileGridCaptionsDialog>().unwrap();
            let flags = CaptionFlags::from_bits_truncate(v[1].get::<u32>().unwrap());
            f(&dlg, flags); None
        })
    }

    fn emit_captions_changed(&self) {
        let bits = self.current_flags().bits();
        self.emit_by_name::<()>("captions-changed", &[&bits.to_value()]);
    }
}

/* icon_helpers.rs
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

//! Native icon helper functions.
//!
//! Nautilus asks GIO for themed icons derived from content types instead of
//! hard-coding icon names.  These helpers do the same so the file views follow
//! the user's current icon theme.

use gtk::gio;
use std::path::Path;

fn content_type_for_path(path: &Path) -> gtk::glib::GString {
    gio::content_type_guess(Some(path), &[]).0
}

/// Returns a native symbolic icon for a file using the current system icon theme.
#[inline]
pub fn file_icon_symbolic(path: &Path) -> gio::Icon {
    let content_type = content_type_for_path(path);
    gio::content_type_get_symbolic_icon(&content_type)
}

/// Returns a native full-color icon for a file using the current system icon theme.
#[inline]
pub fn file_icon(path: &Path) -> gio::Icon {
    let content_type = content_type_for_path(path);
    gio::content_type_get_icon(&content_type)
}

/// Returns a native full-color folder icon using the current system icon theme.
#[inline]
pub fn folder_icon() -> gio::Icon {
    gio::content_type_get_icon("inode/directory")
}

/// Returns a native symbolic folder icon using the current system icon theme.
#[inline]
pub fn folder_icon_symbolic() -> gio::Icon {
    gio::content_type_get_symbolic_icon("inode/directory")
}

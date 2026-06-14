/* views/mod.rs
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

//! File-browser view builders.
//!
//! This module groups the interchangeable view modes for the right-hand
//! file-browser panel:
//!
//! - [`list_view`]      — compact list with file-type badges and chevrons.
//! - [`grid_view`]      — icon grid with 64-px thumbnails and wrapping labels.
//! - [`file_list_view`] — unified GObject widget wrapping both modes with an
//!                        empty-state fallback; used directly in window.rs.

pub mod grid_view;
pub mod list_view;
pub mod file_list_view;
#[allow(unused_imports)]
pub use file_list_view::FileListView;

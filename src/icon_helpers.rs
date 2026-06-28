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

//! Icon-name helper functions.
//!
//! Provides MIME-type and folder icon lookups used by the list and grid
//! views.  Extracted from `window.rs` to eliminate duplication between
//! the two view builders and to keep the icon mapping in one place.

use std::path::Path;

// ── Internal parametric helper ────────────────────────────────────────────────

/// Core MIME icon lookup.
///
/// `symbolic = true`  → appends `-symbolic` to every icon name (list view).
/// `symbolic = false` → returns the full (non-symbolic) name (grid view).
///
/// All callers should use the public wrappers [`mime_icon`] and
/// [`mime_icon_full`] instead of calling this directly.
fn mime_icon_inner(path: &Path, symbolic: bool) -> &'static str {
    let base = match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "text-x-rust",
        Some("toml") | Some("yaml") | Some("yml") => "text-x-script",
        Some("json") => "text-x-script",
        Some("xml") | Some("blp") | Some("ui") => "text-xml",
        Some("md") | Some("rst") | Some("txt") => "text-x-generic",
        Some("png") | Some("jpg") | Some("jpeg") => "image-x-generic",
        Some("svg") | Some("webp") | Some("gif") => "image-x-generic",
        Some("mp3") | Some("ogg") | Some("flac") | Some("wav") => "audio-x-generic",
        Some("mp4") | Some("mkv") | Some("webm") | Some("avi") => "video-x-generic",
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "text-x-script",
        Some("c") | Some("h") | Some("cpp") | Some("hpp") => "text-x-csrc",
        Some("py") => "text-x-python",
        Some("js") | Some("ts") | Some("jsx") | Some("tsx") => "text-x-javascript",
        Some("html") | Some("css") => "text-html",
        Some("pdf") => "application-pdf",
        Some("zip") | Some("tar") | Some("gz") | Some("xz") => "application-zip",
        Some("lock") => "text-x-generic",
        Some("in") => "text-x-makefile",
        _ => match path.file_name().and_then(|n| n.to_str()) {
            Some(".gitignore") | Some(".gitattributes") | Some(".gitmodules") => "text-x-generic",
            Some("Makefile") | Some("makefile") | Some("GNUmakefile") => "text-x-makefile",
            Some("LICENSE") | Some("COPYING") | Some("NOTICE") => "text-x-generic",
            Some("Dockerfile") | Some("Containerfile") => "application-x-executable",
            _ => "text-x-generic",
        },
    };

    if symbolic {
        // SAFETY: every `base` value above is a string literal whose
        // `-symbolic` counterpart is also a valid GTK icon name.
        // We use a second match (same arms) to return the static `&'static str`
        // form so that Rust does not need to allocate a String at runtime.
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => "text-x-rust-symbolic",
            Some("toml") | Some("yaml") | Some("yml") => "text-x-script-symbolic",
            Some("json") => "text-x-script-symbolic",
            Some("xml") | Some("blp") | Some("ui") => "text-xml-symbolic",
            Some("md") | Some("rst") | Some("txt") => "text-x-generic-symbolic",
            Some("png") | Some("jpg") | Some("jpeg") => "image-x-generic-symbolic",
            Some("svg") | Some("webp") | Some("gif") => "image-x-generic-symbolic",
            Some("mp3") | Some("ogg") | Some("flac") | Some("wav") => "audio-x-generic-symbolic",
            Some("mp4") | Some("mkv") | Some("webm") | Some("avi") => "video-x-generic-symbolic",
            Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "text-x-script-symbolic",
            Some("c") | Some("h") | Some("cpp") | Some("hpp") => "text-x-csrc-symbolic",
            Some("py") => "text-x-python-symbolic",
            Some("js") | Some("ts") | Some("jsx") | Some("tsx") => "text-x-javascript-symbolic",
            Some("html") | Some("css") => "text-html-symbolic",
            Some("pdf") => "application-pdf-symbolic",
            Some("zip") | Some("tar") | Some("gz") | Some("xz") => "application-zip-symbolic",
            Some("lock") => "text-x-generic-symbolic",
            Some("in") => "text-x-makefile-symbolic",
            _ => match path.file_name().and_then(|n| n.to_str()) {
                Some(".gitignore") | Some(".gitattributes") | Some(".gitmodules") => {
                    "text-x-generic-symbolic"
                }
                Some("Makefile") | Some("makefile") | Some("GNUmakefile") => {
                    "text-x-makefile-symbolic"
                }
                Some("LICENSE") | Some("COPYING") | Some("NOTICE") => "text-x-generic-symbolic",
                Some("Dockerfile") | Some("Containerfile") => "application-x-executable-symbolic",
                _ => "text-x-generic-symbolic",
            },
        }
    } else {
        base
    }
}

// ── Internal parametric helper (folders) ─────────────────────────────────────

/// Core folder icon lookup.
///
/// `symbolic = true`  → appends `-symbolic` (list view).
/// `symbolic = false` → returns the full name (grid view).
///
/// Note: `_suffix` is intentionally unused as a variable — the actual
/// suffix strings are embedded directly in the `concat!` macro arms below
/// so that each branch returns a `&'static str` without runtime allocation.
fn folder_icon_inner(name: &str, symbolic: bool) -> &'static str {
    let _suffix = if symbolic { "-symbolic" } else { "" };

    // Use a macro so we only write the pattern list once.
    macro_rules! folder_match {
        ($suffix:expr) => {
            match name.to_lowercase().as_str() {
                "src" | "source" | "lib" | "crates" => concat!("folder-development", $suffix),
                "code" | "devel" | "development" | "projects" | "projetos" => {
                    concat!("folder-development", $suffix)
                }
                "doc" | "docs" | "documents" | "documentos" | "documentation" => {
                    concat!("folder-documents", $suffix)
                }
                "data" | "db" | "database" | "datasets" => concat!("folder-documents", $suffix),
                "test" | "tests" | "spec" | "specs" | "testing" => {
                    concat!("folder-remote", $suffix)
                }
                "images" | "img" | "pictures" | "imagens" | "assets" | "media" => {
                    concat!("folder-pictures", $suffix)
                }
                "icons" | "pixmaps" => concat!("folder-pictures", $suffix),
                "videos" | "video" => concat!("folder-videos", $suffix),
                "music" | "audio" | "músicas" | "musicas" | "sounds" => {
                    concat!("folder-music", $suffix)
                }
                "download" | "downloads" => concat!("folder-download", $suffix),
                "build" | "target" | "dist" | "out" | "output" => concat!("folder-remote", $suffix),
                "config" | "cfg" | "settings" | "conf" => concat!("folder-documents", $suffix),
                "scripts" | "bin" | "tools" => concat!("folder-development", $suffix),
                "po" | "i18n" | "l10n" | "locale" => concat!("folder-documents", $suffix),
                "themes" | "theme" | "skins" => concat!("folder-pictures", $suffix),
                _ => concat!("folder", $suffix),
            }
        };
    }

    if symbolic {
        folder_match!("-symbolic")
    } else {
        folder_match!("")
    }
}

// ── Public API — MIME icons ───────────────────────────────────────────────────

/// Returns a **symbolic** icon name for a file based on its extension or
/// filename.  Used by the list view.
#[inline]
pub fn mime_icon(path: &Path) -> &'static str {
    mime_icon_inner(path, true)
}

/// Returns a **full** (non-symbolic) icon name for a file.  Used by the
/// grid view.
#[inline]
pub fn mime_icon_full(path: &Path) -> &'static str {
    mime_icon_inner(path, false)
}

// ── Public API — Folder icons ─────────────────────────────────────────────────

/// Returns a **full** folder icon name based on a well-known directory name.
#[inline]
pub fn folder_icon(name: &str) -> &'static str {
    folder_icon_inner(name, false)
}

/// Returns a **symbolic** folder icon name based on a well-known directory
/// name.
#[inline]
pub fn folder_icon_symbolic(name: &str) -> &'static str {
    folder_icon_inner(name, true)
}

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

// ── MIME icons (symbolic — for list view) ────────────────────────────────────

/// Returns a symbolic icon name for a file based on its extension or filename.
pub fn mime_icon(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")                                              => "text-x-rust-symbolic",
        Some("toml") | Some("yaml") | Some("yml")              => "text-x-script-symbolic",
        Some("json")                                            => "text-x-script-symbolic",
        Some("xml") | Some("blp") | Some("ui")                 => "text-xml-symbolic",
        Some("md") | Some("rst") | Some("txt")                 => "text-x-generic-symbolic",
        Some("png") | Some("jpg") | Some("jpeg")               => "image-x-generic-symbolic",
        Some("svg") | Some("webp") | Some("gif")               => "image-x-generic-symbolic",
        Some("mp3") | Some("ogg") | Some("flac") | Some("wav") => "audio-x-generic-symbolic",
        Some("mp4") | Some("mkv") | Some("webm") | Some("avi") => "video-x-generic-symbolic",
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "text-x-script-symbolic",
        Some("c") | Some("h") | Some("cpp") | Some("hpp")      => "text-x-csrc-symbolic",
        Some("py")                                             => "text-x-python-symbolic",
        Some("js") | Some("ts") | Some("jsx") | Some("tsx")    => "text-x-javascript-symbolic",
        Some("html") | Some("css")                             => "text-html-symbolic",
        Some("pdf")                                            => "application-pdf-symbolic",
        Some("zip") | Some("tar") | Some("gz") | Some("xz")    => "application-zip-symbolic",
        Some("lock")                                           => "text-x-generic-symbolic",
        Some("in")                                             => "text-x-makefile-symbolic",
        _ => match path.file_name().and_then(|n| n.to_str()) {
            Some(".gitignore") | Some(".gitattributes") | Some(".gitmodules") => "text-x-generic-symbolic",
            Some("Makefile") | Some("makefile") | Some("GNUmakefile")         => "text-x-makefile-symbolic",
            Some("LICENSE") | Some("COPYING") | Some("NOTICE")               => "text-x-generic-symbolic",
            Some("Dockerfile") | Some("Containerfile")                       => "application-x-executable-symbolic",
            _                                                                => "text-x-generic-symbolic",
        },
    }
}

// ── MIME icons (full — for grid view) ────────────────────────────────────────

/// Returns a full (non-symbolic) icon name for a file, used by the grid view.
pub fn mime_icon_full(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")                                              => "text-x-rust",
        Some("toml") | Some("yaml") | Some("yml")              => "text-x-script",
        Some("json")                                            => "text-x-script",
        Some("xml") | Some("blp") | Some("ui")                 => "text-xml",
        Some("md") | Some("rst") | Some("txt")                 => "text-x-generic",
        Some("png") | Some("jpg") | Some("jpeg")               => "image-x-generic",
        Some("svg") | Some("webp") | Some("gif")               => "image-x-generic",
        Some("mp3") | Some("ogg") | Some("flac") | Some("wav") => "audio-x-generic",
        Some("mp4") | Some("mkv") | Some("webm") | Some("avi") => "video-x-generic",
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "text-x-script",
        Some("c") | Some("h") | Some("cpp") | Some("hpp")      => "text-x-csrc",
        Some("py")                                             => "text-x-python",
        Some("js") | Some("ts") | Some("jsx") | Some("tsx")    => "text-x-javascript",
        Some("html") | Some("css")                             => "text-html",
        Some("pdf")                                            => "application-pdf",
        Some("zip") | Some("tar") | Some("gz") | Some("xz")    => "application-zip",
        Some("lock")                                           => "text-x-generic",
        Some("in")                                             => "text-x-makefile",
        _ => match path.file_name().and_then(|n| n.to_str()) {
            Some(".gitignore") | Some(".gitattributes") | Some(".gitmodules") => "text-x-generic",
            Some("Makefile") | Some("makefile") | Some("GNUmakefile")         => "text-x-makefile",
            Some("LICENSE") | Some("COPYING") | Some("NOTICE")               => "text-x-generic",
            Some("Dockerfile") | Some("Containerfile")                       => "application-x-executable",
            _                                                                => "text-x-generic",
        },
    }
}

// ── Folder icons (full — for grid view) ──────────────────────────────────────

/// Returns a full folder icon name based on a well-known directory name.
pub fn folder_icon(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "src" | "source" | "lib" | "crates"                              => "folder-development",
        "code" | "devel" | "development" | "projects" | "projetos"       => "folder-development",
        "doc" | "docs" | "documents" | "documentos" | "documentation"    => "folder-documents",
        "data" | "db" | "database" | "datasets"                          => "folder-documents",
        "test" | "tests" | "spec" | "specs" | "testing"                  => "folder-remote",
        "images" | "img" | "pictures" | "imagens" | "assets" | "media"   => "folder-pictures",
        "icons" | "pixmaps"                                              => "folder-pictures",
        "videos" | "video"                                               => "folder-videos",
        "music" | "audio" | "músicas" | "musicas" | "sounds"             => "folder-music",
        "download" | "downloads"                                         => "folder-download",
        "build" | "target" | "dist" | "out" | "output"                   => "folder-remote",
        "config" | "cfg" | "settings" | "conf"                           => "folder-documents",
        "scripts" | "bin" | "tools"                                      => "folder-development",
        "po" | "i18n" | "l10n" | "locale"                               => "folder-documents",
        "themes" | "theme" | "skins"                                     => "folder-pictures",
        _                                                                => "folder",
    }
}

/// Returns a symbolic folder icon name based on a well-known directory name.
pub fn folder_icon_symbolic(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "src" | "source" | "lib" | "crates"                              => "folder-development-symbolic",
        "code" | "devel" | "development" | "projects" | "projetos"       => "folder-development-symbolic",
        "doc" | "docs" | "documents" | "documentos" | "documentation"    => "folder-documents-symbolic",
        "data" | "db" | "database" | "datasets"                          => "folder-documents-symbolic",
        "test" | "tests" | "spec" | "specs" | "testing"                  => "folder-remote-symbolic",
        "images" | "img" | "pictures" | "imagens" | "assets" | "media"   => "folder-pictures-symbolic",
        "icons" | "pixmaps"                                              => "folder-pictures-symbolic",
        "videos" | "video"                                               => "folder-videos-symbolic",
        "music" | "audio" | "músicas" | "musicas" | "sounds"             => "folder-music-symbolic",
        "download" | "downloads"                                         => "folder-download-symbolic",
        "build" | "target" | "dist" | "out" | "output"                   => "folder-remote-symbolic",
        "config" | "cfg" | "settings" | "conf"                           => "folder-documents-symbolic",
        "scripts" | "bin" | "tools"                                      => "folder-development-symbolic",
        "po" | "i18n" | "l10n" | "locale"                               => "folder-documents-symbolic",
        "themes" | "theme" | "skins"                                     => "folder-pictures-symbolic",
        _                                                                => "folder-symbolic",
    }
}

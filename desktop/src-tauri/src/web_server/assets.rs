//! The GUI files the headless server hands to the browser: either the frontend embedded in the binary
//! by Tauri (the same files the desktop window shows) or a directory such as `desktop/build`
//! (`--web-root`). `index.html` is rewritten on the way out: Tauri's own CSP tag is removed, the web shim
//! is injected first in `<head>`, and a strict CSP header allows only the page's own inline bootstrap
//! scripts, by hash.

use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

pub(crate) const SHIM_PATH: &str = "/__hevc/web-shim.js";
pub(crate) const SHIM_SOURCE: &str = include_str!("web_shim.js");

pub(crate) enum AssetSource {
    Directory(PathBuf),
    Memory(HashMap<String, Vec<u8>>),
}

pub(crate) struct Asset {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: &'static str,
}

impl AssetSource {
    /// Safe relative lookup: no parent or absolute components, so a request cannot leave the root.
    pub(crate) fn read(&self, request_path: &str) -> Option<Vec<u8>> {
        let relative = request_path.trim_start_matches('/');
        let relative = if relative.is_empty() {
            "index.html"
        } else {
            relative
        };
        let path = Path::new(relative);
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return None;
        }
        match self {
            AssetSource::Directory(root) => {
                let file = root.join(path);
                if file.is_file() {
                    std::fs::read(file).ok()
                } else {
                    None
                }
            }
            AssetSource::Memory(files) => files.get(relative).cloned(),
        }
    }

    pub(crate) fn has_index(&self) -> bool {
        self.read("index.html").is_some()
    }

    /// Resolve a request path to a file; extensionless paths are client-side routes and get `index.html`.
    pub(crate) fn resolve(&self, request_path: &str) -> Option<Asset> {
        let request_path = if request_path.trim_matches('/').is_empty() {
            "/index.html"
        } else {
            request_path
        };
        let direct = self
            .read(request_path)
            .map(|bytes| (bytes, content_type(request_path)));
        let (bytes, kind) = match direct {
            Some(found) => found,
            None if Path::new(request_path).extension().is_none() => {
                (self.read("index.html")?, "text/html; charset=utf-8")
            }
            None => return None,
        };
        let is_index = kind.starts_with("text/html");
        Some(Asset {
            bytes: if is_index {
                prepare_index(&String::from_utf8_lossy(&bytes)).into_bytes()
            } else {
                bytes
            },
            content_type: kind,
        })
    }
}

pub(crate) fn content_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") | Some("webmanifest") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(TABLE[(triple >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// CSP source expressions (`'sha256-…'`) for every inline `<script>` in the page.
pub(crate) fn inline_script_hashes(html: &str) -> Vec<String> {
    let scripts = Regex::new(r"(?is)<script((?:\s[^>]*)?)>(.*?)</script>").expect("script pattern");
    scripts
        .captures_iter(html)
        .filter(|captures| !captures[1].to_ascii_lowercase().contains("src="))
        .map(|captures| {
            format!(
                "'sha256-{}'",
                base64(&Sha256::digest(captures[2].as_bytes()))
            )
        })
        .collect()
}

pub(crate) fn content_security_policy(hashes: &[String]) -> String {
    let scripts = if hashes.is_empty() {
        String::new()
    } else {
        format!(" {}", hashes.join(" "))
    };
    format!(
        "default-src 'self'; script-src 'self'{scripts}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob: https://image.tmdb.org https://static.tvmaze.com https://upload.wikimedia.org https://www.themoviedb.org; \
         font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'"
    )
}

/// The page the browser gets: Tauri's CSP tag out (it only allows `ipc:`), the shim in first.
pub(crate) fn prepare_index(html: &str) -> String {
    let csp_tag =
        Regex::new(r#"(?is)<meta\s[^>]*http-equiv\s*=\s*["']?Content-Security-Policy["']?[^>]*>"#)
            .expect("csp pattern");
    let stripped = csp_tag.replace_all(html, "");
    let shim = format!(r#"<script src="{SHIM_PATH}"></script>"#);
    let head = Regex::new(r"(?i)<head[^>]*>").expect("head pattern");
    match head.find(&stripped) {
        Some(found) => format!(
            "{}{}{}",
            &stripped[..found.end()],
            shim,
            &stripped[found.end()..]
        ),
        None => format!("{shim}{stripped}"),
    }
}

/// Embedded frontend files by relative path, decompressed. Empty when this build did not embed a
/// frontend (a plain debug build uses the dev server instead), and always empty in the headless build,
/// which serves the GUI from a folder (`--web-root`, or `web/` next to the executable).
#[cfg(feature = "desktop")]
pub(crate) fn embedded_assets() -> HashMap<String, Vec<u8>> {
    let context = crate::tauri_context();
    let assets = context.assets();
    let mut files = HashMap::new();
    for (key, _) in assets.iter() {
        let key = key.into_owned();
        if let Some(bytes) = assets.get(&tauri::utils::assets::AssetKey::from(key.as_str())) {
            files.insert(key.trim_start_matches('/').to_string(), bytes.into_owned());
        }
    }
    files
}

#[cfg(not(feature = "desktop"))]
pub(crate) fn embedded_assets() -> HashMap<String, Vec<u8>> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'self'; connect-src ipc:"><script type="module" src="/_app/start.js"></script></head><body><script>window.boot = 1;</script></body></html>"#;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn index_loses_the_tauri_csp_tag_and_gains_the_shim_first() {
        let page = prepare_index(PAGE);
        assert!(!page.to_lowercase().contains("content-security-policy"));
        let head = page.find("<head>").unwrap() + "<head>".len();
        assert!(page[head..].starts_with(r#"<script src="/__hevc/web-shim.js"></script>"#));
        assert!(page.contains("/_app/start.js"));
    }

    #[test]
    fn only_inline_scripts_are_hashed() {
        let hashes = inline_script_hashes(PAGE);
        assert_eq!(hashes.len(), 1, "{hashes:?}");
        let expected = format!("'sha256-{}'", base64(&Sha256::digest(b"window.boot = 1;")));
        assert_eq!(hashes[0], expected);
        let policy = content_security_policy(&hashes);
        assert!(
            policy.contains(&expected)
                && policy.contains("connect-src 'self'")
                && policy.contains("frame-ancestors 'none'")
        );
        assert!(
            !policy.contains("unsafe-eval")
                && !policy.contains("script-src 'self' 'unsafe-inline'")
        );
    }

    #[test]
    fn memory_source_serves_files_and_client_routes_but_not_traversal() {
        let mut files = HashMap::new();
        files.insert("index.html".to_string(), PAGE.as_bytes().to_vec());
        files.insert("_app/start.js".to_string(), b"console.log(1)".to_vec());
        let source = AssetSource::Memory(files);
        assert!(source.has_index());
        assert_eq!(
            source.resolve("/_app/start.js").unwrap().content_type,
            "text/javascript; charset=utf-8"
        );
        let route = source
            .resolve("/library")
            .expect("extensionless path is a client route");
        assert!(String::from_utf8(route.bytes)
            .unwrap()
            .contains("web-shim.js"));
        assert!(source.resolve("/missing.js").is_none());
        assert!(source.read("/../etc/passwd").is_none());
        assert!(source.read("/_app/../index.html").is_none());
    }

    #[test]
    fn directory_source_reads_from_disk_and_refuses_escapes() {
        let root = crate::test_dir("web-assets");
        std::fs::write(root.join("index.html"), PAGE).unwrap();
        std::fs::write(root.join("secret.txt"), "no").unwrap();
        let inner = root.join("site");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("index.html"), PAGE).unwrap();
        let source = AssetSource::Directory(inner);
        assert!(source.has_index());
        assert!(source.read("/../secret.txt").is_none());
        assert!(source.read("/%2e%2e/secret.txt").is_none());
        std::fs::remove_dir_all(root).ok();
    }
}

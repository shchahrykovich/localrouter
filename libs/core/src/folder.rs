//! Folder routes: the daemon serves the files of a folder itself, with no dev
//! server. The target is `file://` plus the folder (see `routes.rs`).
//!
//! The request path is resolved here, not by tower-http: `ServeFile` only gets
//! a file that is known to be inside the folder. It adds the content type,
//! `Range`, `If-Modified-Since` and `HEAD`. Names that start with `.` are never
//! served (`.git`, `.env`), and a symlink that leads out of the folder answers
//! 404 like a missing file.

use std::io;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use tower_http::services::ServeFile;

use crate::instance::Instance;
use crate::proxy::{Body, escape, page};
use crate::routes::Route;

/// Characters a file name must not carry as is in a relative link.
const LINK: &AsciiSet =
    &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'\\').add(b'/');

/// What a request path leads to in the folder.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
    /// A file inside the folder: its real path.
    File(PathBuf),
    /// A folder without `index.html`: its entries, folders first.
    Listing(Vec<Entry>),
    /// A folder asked for without the trailing `/`. Relative links in its
    /// pages need the slash, so the browser is sent there.
    AddSlash,
    /// Missing, hidden, or outside the folder.
    NotFound,
    /// The system refused to read it.
    Denied(String),
    /// The route's folder itself is missing or cannot be read.
    FolderGone(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// `.git`, `.env`, `..`: never served, never listed.
fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.contains(['\\', '\0'])
}

/// The real path of `path` when it is inside `root` and no part of it below
/// `root` is hidden. `Ok(None)` when it does not exist or leads out.
fn inside(root: &Path, path: &Path) -> io::Result<Option<PathBuf>> {
    let real = match path.canonicalize() {
        Ok(real) => real,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
        Err(_) => return Ok(None),
    };
    let ok = real.strip_prefix(root).is_ok_and(|rest| !rest.iter().any(|c| hidden(&c.to_string_lossy())));
    Ok(ok.then_some(real))
}

fn denied(e: io::Error) -> Resolved {
    Resolved::Denied(e.to_string())
}

/// Map `rel`, the request path below the route path (still percent-encoded,
/// without the query), to what the folder holds. Reads the disk.
pub fn resolve(folder: &Path, rel: &str) -> Resolved {
    let root = match folder.canonicalize() {
        Ok(root) if root.is_dir() => root,
        Ok(_) => return Resolved::FolderGone("it is not a folder".into()),
        // macOS privacy rules refuse with EPERM: the folder is there.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return denied(e),
        Err(e) => return Resolved::FolderGone(e.to_string()),
    };
    let Ok(decoded) = percent_decode_str(rel).decode_utf8() else { return Resolved::NotFound };
    let mut path = root.clone();
    for segment in decoded.split('/').filter(|s| !s.is_empty()) {
        if hidden(segment) {
            return Resolved::NotFound;
        }
        path.push(segment);
    }
    let real = match inside(&root, &path) {
        Ok(Some(real)) => real,
        Ok(None) => return Resolved::NotFound,
        Err(e) => return denied(e),
    };
    if !real.is_dir() {
        // ServeFile answers a file it may not open with an empty 404; open it
        // here first so the answer names the reason.
        return match std::fs::File::open(&real) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied(e),
            _ => Resolved::File(real),
        };
    }
    if !rel.ends_with('/') {
        return Resolved::AddSlash;
    }
    match inside(&root, &real.join("index.html")) {
        Ok(Some(index)) if index.is_file() => {
            return match std::fs::File::open(&index) {
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied(e),
                _ => Resolved::File(index),
            };
        }
        Err(e) => return denied(e),
        _ => {}
    }
    let read = match std::fs::read_dir(&real) {
        Ok(read) => read,
        Err(e) => return denied(e),
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok().filter(|n| !hidden(n))?;
            // Only what a click can open: a link out of the folder is left out.
            let target = inside(&root, &e.path()).ok()??;
            Some(Entry { name, is_dir: target.is_dir() })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Resolved::Listing(entries)
}

/// Answer a request of a folder route. `host` is the `Host` header, for the
/// pages.
pub async fn serve<B: Send + 'static>(
    req: Request<B>,
    instance: &Instance,
    route: &Route,
    folder: &Path,
    host: &str,
) -> Response<Body> {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let mut resp = page(
            StatusCode::METHOD_NOT_ALLOWED,
            "Only GET and HEAD",
            "<p>This name serves the files of a folder. It only answers GET and HEAD.</p>".into(),
        );
        resp.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return resp;
    }
    let path = req.uri().path().to_string();
    // A folder route always maps its path to the folder: /docs/a.html is a.html.
    let rel = match route.path.as_deref() {
        Some(prefix) => path.strip_prefix(prefix).unwrap_or(&path).to_string(),
        None => path.clone(),
    };
    let at_root = rel.is_empty() || rel == "/";
    let root = folder.to_path_buf();
    let resolved =
        tokio::task::spawn_blocking(move || resolve(&root, &rel)).await.unwrap_or_else(|e| Resolved::Denied(e.to_string()));
    let shown = || escape(&format!("{host}{path}"));
    let folder_html = || escape(&folder.display().to_string());
    match resolved {
        Resolved::File(file) => serve_file(req, &file).await,
        Resolved::Listing(entries) => listing(&path, &entries, at_root),
        Resolved::AddSlash => {
            let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
            let location = HeaderValue::from_str(&format!("{path}/{query}")).unwrap_or(HeaderValue::from_static("/"));
            Response::builder()
                .status(StatusCode::PERMANENT_REDIRECT)
                .header(header::LOCATION, location)
                .body(Full::new(Bytes::new()).map_err(|never| match never {}).boxed_unsync())
                .expect("static response parts are valid")
        }
        Resolved::NotFound => page(
            StatusCode::NOT_FOUND,
            "No such file",
            format!(
                "<p><code>{}</code> is not a file in <code>{}</code>.</p>\
                 <p>Names that start with a dot are not served, and neither are links that lead out of the folder.</p>",
                shown(),
                folder_html()
            ),
        ),
        Resolved::Denied(why) => page(
            StatusCode::FORBIDDEN,
            "Cannot read this file",
            format!(
                "<p><code>{}</code> is in <code>{}</code>, but reading it failed: {}.</p>\
                 <p>If the folder is in Desktop, Documents, Downloads or iCloud Drive, macOS keeps apps out of it. \
                 Use a folder outside them, or allow {} in System Settings &gt; Privacy &amp; Security.</p>",
                shown(),
                folder_html(),
                escape(&why),
                escape(&instance.app_name())
            ),
        ),
        Resolved::FolderGone(why) => {
            let note = if route.note.is_empty() { String::new() } else { format!("<p>Note: {}</p>", escape(&route.note)) };
            page(
                StatusCode::BAD_GATEWAY,
                "The folder is not there",
                format!(
                    "<p><code>{}</code> serves the folder <code>{}</code>, but it cannot be opened: {}.</p>{note}",
                    escape(&format!("{host}{}", route.path.as_deref().unwrap_or(""))),
                    folder_html(),
                    escape(&why)
                ),
            )
        }
    }
}

async fn serve_file<B: Send + 'static>(req: Request<B>, file: &Path) -> Response<Body> {
    match ServeFile::new(file).try_call(req).await {
        Ok(resp) => {
            let (mut parts, body) = resp.into_parts();
            let h = &mut parts.headers;
            // Text without a charset is read as Latin-1 by some browsers.
            if let Some(ct) = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok())
                && (ct.starts_with("text/") || ct == "application/javascript")
                && !ct.contains("charset")
                && let Ok(v) = HeaderValue::from_str(&format!("{ct}; charset=utf-8"))
            {
                h.insert(header::CONTENT_TYPE, v);
            }
            // The files change while you work: the browser asks every time and
            // gets 304 while the file is the same.
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            Response::from_parts(parts, body.map_err(Into::into).boxed_unsync())
        }
        Err(e) => page(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot read this file",
            format!("<p><code>{}</code>: {}</p>", escape(&file.display().to_string()), escape(&e.to_string())),
        ),
    }
}

fn listing(path: &str, entries: &[Entry], at_root: bool) -> Response<Body> {
    let mut items = String::new();
    if !at_root {
        items.push_str("<li><a href=\"../\">../</a></li>");
    }
    for e in entries {
        let slash = if e.is_dir { "/" } else { "" };
        let href = utf8_percent_encode(&e.name, LINK).to_string();
        items.push_str(&format!("<li><a href=\"{}{slash}\">{}{slash}</a></li>", escape(&href), escape(&e.name)));
    }
    if entries.is_empty() {
        items.push_str("<li>This folder is empty.</li>");
    }
    let title = format!("Index of {}", escape(&percent_decode_str(path).decode_utf8_lossy()));
    let mut resp = page(StatusCode::OK, &title, format!("<ul>{items}</ul><p>Add an index.html to show a page here.</p>"));
    // `page` is for errors; a listing may be cached like a file.
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;

    /// site/ with index.html, a.txt, sub/ (no index), .git/config, .env,
    /// and links: out -> the parent folder, in -> a.txt, dot -> .git.
    fn site() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("site");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("index.html"), "<h1>home</h1>").unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("sub/b c.html"), "b").unwrap();
        fs::write(root.join(".git/config"), "secret").unwrap();
        fs::write(root.join(".env"), "secret").unwrap();
        fs::write(dir.path().join("outside.txt"), "secret").unwrap();
        symlink(dir.path(), root.join("out")).unwrap();
        symlink(root.join("a.txt"), root.join("in")).unwrap();
        symlink(root.join(".git"), root.join("dot")).unwrap();
        (dir, root)
    }

    fn real(p: PathBuf) -> PathBuf {
        p.canonicalize().unwrap()
    }

    #[test]
    fn files_folders_and_index_html() {
        let (_dir, root) = site();
        assert_eq!(resolve(&root, "/"), Resolved::File(real(root.join("index.html"))));
        assert_eq!(resolve(&root, "/a.txt"), Resolved::File(real(root.join("a.txt"))));
        assert_eq!(resolve(&root, "/sub/b%20c.html"), Resolved::File(real(root.join("sub/b c.html"))));
        assert_eq!(resolve(&root, "/in"), Resolved::File(real(root.join("a.txt"))), "a link inside the folder is served");
        assert_eq!(resolve(&root, "/sub"), Resolved::AddSlash);
        assert_eq!(resolve(&root, ""), Resolved::AddSlash, "a path route asked without its slash");
        assert_eq!(resolve(&root, "/nope.html"), Resolved::NotFound);
        assert_eq!(resolve(&root, "/a.txt/x"), Resolved::NotFound);
    }

    #[test]
    fn a_folder_without_index_html_is_listed_without_hidden_names() {
        let (_dir, root) = site();
        let Resolved::Listing(entries) = resolve(&root, "/sub/") else { panic!("no listing") };
        assert_eq!(entries, [Entry { name: "b c.html".into(), is_dir: false }]);
        fs::remove_file(root.join("index.html")).unwrap();
        let Resolved::Listing(entries) = resolve(&root, "/") else { panic!("no listing") };
        let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
        assert_eq!(names, [("sub", true), ("a.txt", false), ("in", false)], "links out and to .git are left out");
    }

    #[test]
    fn hidden_names_dot_dot_and_links_out_of_the_folder_are_not_found() {
        let (_dir, root) = site();
        for rel in [
            "/.env",
            "/.git/config",
            "/%2egit/config",
            "/../outside.txt",
            "/sub/../../outside.txt",
            "/%2e%2e/outside.txt",
            "/sub%2f..%2f..%2foutside.txt",
            "/out/outside.txt",
            "/dot/config",
            "/a.txt%00",
            "/%ff",
        ] {
            assert_eq!(resolve(&root, rel), Resolved::NotFound, "{rel}");
        }
    }

    #[test]
    fn a_missing_folder_is_reported_as_gone() {
        let (dir, root) = site();
        assert!(matches!(resolve(&dir.path().join("nope"), "/"), Resolved::FolderGone(_)));
        assert!(matches!(resolve(&root.join("a.txt"), "/"), Resolved::FolderGone(why) if why.contains("not a folder")));
    }

    /// macOS privacy rules refuse with EPERM, which Rust reports as
    /// PermissionDenied, like a folder without the search permission.
    #[test]
    fn a_folder_the_system_refuses_is_denied_not_gone() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, root) = site();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o000)).unwrap();
        let got = resolve(&root, "/");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(got, Resolved::Denied(_)), "{got:?}");
    }

    #[test]
    fn listing_links_are_encoded_and_escaped() {
        let entries = [Entry { name: "a b#<c>.html".into(), is_dir: false }, Entry { name: "d".into(), is_dir: true }];
        let resp = listing("/docs/x%20y/", &entries, false);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let body = rt.block_on(resp.into_body().collect()).unwrap().to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<a href=\"a%20b%23%3Cc%3E.html\">a b#&lt;c&gt;.html</a>"), "{html}");
        assert!(html.contains("<a href=\"d/\">d/</a>"), "{html}");
        assert!(html.contains("<a href=\"../\">"), "{html}");
        assert!(html.contains("Index of /docs/x y/"), "{html}");
    }
}

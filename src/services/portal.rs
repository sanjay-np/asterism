//! The desktop's own dialogs, through xdg-desktop-portal on the session bus: here, choosing a
//! folder. The portal answers on a Request object whose path follows from our name and a
//! token we pick, so the answer is listened for before the question is asked.

use super::dbus;
use futures_lite::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

/// Asks for a folder with the desktop's file dialog, starting in `start`. Nothing when it's
/// cancelled, when there's no portal, or after ten minutes.
pub async fn choose_folder(bus: &Connection, title: &str, start: &Path) -> Option<PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let token = format!("asterism_{}_{nanos}", std::process::id());
    let sender = bus.unique_name()?.as_str().trim_start_matches(':').replace('.', "_");
    let handle = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let rule = dbus::rule().interface("org.freedesktop.portal.Request").ok()?.member("Response").ok()?.path(handle).ok()?.build();
    let mut answers = dbus::signals(bus, rule).await?;

    let mut start_bytes = start.as_os_str().as_encoded_bytes().to_vec();
    start_bytes.push(0);
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token));
    options.insert("directory", Value::from(true));
    options.insert("modal", Value::from(true));
    options.insert("accept_label", Value::from("Use this folder"));
    options.insert("current_folder", Value::from(start_bytes));
    let asked = bus.call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.FileChooser"), "OpenFile", &("", title, options)).await;
    if let Err(error) = asked {
        log::warn!("no folder dialog: {error}");
        return None;
    }
    let message = tokio::time::timeout(Duration::from_secs(600), answers.next()).await.ok()??.ok()?;
    let (response, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().ok()?;
    if response != 0 {
        return None;
    }
    let uris = results.get("uris").map(|v| dbus::as_strings(v)).unwrap_or_default();
    uris.first().and_then(|uri| path_of(uri))
}

/// The path in a `file://` URI, its %-escapes undone.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // "file://host/path": only the local host, written or not
    let rest = if rest.starts_with('/') { rest } else { &rest[rest.find('/')?..] };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_file_uris() {
        assert_eq!(path_of("file:///home/me/Pictures/Wallpapers"), Some(PathBuf::from("/home/me/Pictures/Wallpapers")));
        assert_eq!(path_of("file:///home/me/My%20Pictures/%C3%A9t%C3%A9"), Some(PathBuf::from("/home/me/My Pictures/été")));
        assert_eq!(path_of("file://localhost/tmp/a"), Some(PathBuf::from("/tmp/a")));
        assert_eq!(path_of("file:///tmp/100%"), Some(PathBuf::from("/tmp/100%")), "a lone % stays");
        assert_eq!(path_of("https://example.org/a"), None);
    }
}

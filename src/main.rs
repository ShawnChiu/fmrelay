use std::path::PathBuf;
use std::process::ExitCode;

use url::Url;
use zbus::blocking::{connection, fdo::DBusProxy};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::interface;
use zbus::names::BusName;

const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";

struct FileManager;

impl FileManager {
    fn handle(&self, method: &str, uris: Vec<String>, startup_id: String) {
        let paths: Vec<PathBuf> = uris.iter().filter_map(|u| uri_to_path(u)).collect();
        eprintln!("{method} {paths:?} startup_id={startup_id:?}");
    }
}

#[interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.handle("ShowFolders", uris, startup_id);
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.handle("ShowItems", uris, startup_id);
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.handle("ShowItemProperties", uris, startup_id);
    }
}

/// Turns a `file://` URI into a local path; logs and skips anything else.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = Url::parse(uri)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok());
    if path.is_none() {
        eprintln!("fmrelay: skipping {uri:?}: not a local file URI");
    }
    path
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fmrelay: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> zbus::Result<ExitCode> {
    // Serve the object before taking the name so no call arrives unhandled.
    let conn = connection::Builder::session()?
        .serve_at(PATH, FileManager)?
        .build()?;

    let dbus = DBusProxy::new(&conn)?;
    let acquired = dbus.receive_name_acquired()?;

    // No AllowReplacement: a file manager started later must not take the name back.
    // No DoNotQueue: Nautilus doesn't allow replacement either, so if it got there first
    // we wait in the bus queue and take over when it exits.
    let flags = RequestNameFlags::ReplaceExisting.into();
    match conn.request_name_with_flags(NAME, flags)? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => {}
        RequestNameReply::InQueue | RequestNameReply::Exists => {
            let owner = dbus
                .get_name_owner(BusName::try_from(NAME)?)
                .map(|o| o.to_string())
                .unwrap_or_else(|e| format!("unknown ({e})"));
            eprintln!("fmrelay: {NAME} is owned by {owner}, waiting in queue");
        }
    }

    for signal in acquired {
        if signal.args()?.name == NAME {
            eprintln!("fmrelay: serving {NAME}");
        }
    }
    eprintln!("fmrelay: connection to the bus closed");
    Ok(ExitCode::FAILURE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_to_path_decodes_percent_encoding() {
        let path = uri_to_path("file:///home/u/%E4%B8%AD%E6%96%87%20%E6%AA%94.txt");
        assert_eq!(path, Some(PathBuf::from("/home/u/中文 檔.txt")));
    }

    #[test]
    fn uri_to_path_decodes_encoded_slash() {
        assert_eq!(uri_to_path("file:///a%2Fb"), Some(PathBuf::from("/a/b")));
    }

    #[test]
    fn uri_to_path_accepts_localhost() {
        let path = uri_to_path("file://localhost/tmp/x");
        assert_eq!(path, Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn uri_to_path_rejects_other_schemes() {
        assert_eq!(uri_to_path("sftp://host/tmp/x"), None);
        assert_eq!(uri_to_path("file://otherhost/tmp/x"), None);
        assert_eq!(uri_to_path("not a uri"), None);
    }
}

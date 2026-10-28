use std::process::ExitCode;

use zbus::blocking::{connection, fdo::DBusProxy};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::interface;
use zbus::names::BusName;

const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";

struct FileManager;

#[interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        eprintln!("ShowFolders {uris:?} startup_id={startup_id:?}");
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        eprintln!("ShowItems {uris:?} startup_id={startup_id:?}");
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        eprintln!("ShowItemProperties {uris:?} startup_id={startup_id:?}");
    }
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

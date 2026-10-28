use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use url::Url;
use zbus::blocking::{connection, fdo::DBusProxy};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::interface;
use zbus::names::BusName;

const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";

const USAGE: &str = "\
usage: fmrelay --folders CMD [--items CMD] [--properties CMD]

Serve org.freedesktop.FileManager1 and run CMD for each request. CMD is split
like a shell command line (no shell is run) and must contain {} (the path) or
{parent} (its parent folder).

  --folders CMD     for ShowFolders
  --items CMD       for ShowItems (default: --folders)
  --properties CMD  for ShowItemProperties (default: --items)";

/// One command template per method, already split into arguments.
#[derive(Debug, PartialEq)]
struct FileManager {
    folders: Vec<String>,
    items: Vec<String>,
    properties: Vec<String>,
}

impl FileManager {
    fn handle(&self, method: &str, template: &[String], uris: Vec<String>, startup_id: String) {
        let paths: Vec<PathBuf> = uris.iter().filter_map(|u| uri_to_path(u)).collect();
        eprintln!("{method} {template:?} {paths:?} startup_id={startup_id:?}");
    }
}

#[interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.handle("ShowFolders", &self.folders, uris, startup_id);
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.handle("ShowItems", &self.items, uris, startup_id);
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.handle("ShowItemProperties", &self.properties, uris, startup_id);
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

/// Parses the arguments after the program name; `Ok(None)` means `--help`.
fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<FileManager>, lexopt::Error> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_args(args);
    let (mut folders, mut items, mut properties) = (None, None, None);
    while let Some(arg) = parser.next()? {
        let (option, slot) = match arg {
            Long("folders") => ("--folders", &mut folders),
            Long("items") => ("--items", &mut items),
            Long("properties") => ("--properties", &mut properties),
            Short('h') | Long("help") => return Ok(None),
            _ => return Err(arg.unexpected()),
        };
        let command = parser.value()?.string()?;
        *slot = Some(parse_template(option, &command)?);
    }

    let folders = folders.ok_or("missing required option --folders")?;
    let items = items.unwrap_or_else(|| folders.clone());
    let properties = properties.unwrap_or_else(|| items.clone());
    Ok(Some(FileManager {
        folders,
        items,
        properties,
    }))
}

/// Splits `command` into arguments and makes sure it has somewhere to put the paths.
fn parse_template(option: &str, command: &str) -> Result<Vec<String>, String> {
    let template = shlex::split(command)
        .ok_or_else(|| format!("{option}: unbalanced quotes in {command:?}"))?;
    // Appending the paths silently would guess wrong for commands that want them elsewhere.
    if !template
        .iter()
        .any(|arg| arg.contains("{}") || arg.contains("{parent}"))
    {
        return Err(format!("{option}: no {{}} or {{parent}} in {command:?}"));
    }
    Ok(template)
}

fn main() -> ExitCode {
    let fm = match parse_args(std::env::args_os().skip(1)) {
        Ok(Some(fm)) => fm,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("fmrelay: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match run(fm) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fmrelay: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(fm: FileManager) -> zbus::Result<ExitCode> {
    // Serve the object before taking the name so no call arrives unhandled.
    let conn = connection::Builder::session()?
        .serve_at(PATH, fm)?
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

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_args_defaults_items_to_folders_and_properties_to_items() {
        let fm = parse_args(args(&["--folders", "yazi {}"]))
            .unwrap()
            .unwrap();
        assert_eq!(fm.items, strings(&["yazi", "{}"]));
        assert_eq!(fm.properties, strings(&["yazi", "{}"]));

        let fm = parse_args(args(&["--folders=a {}", "--items", "b {parent}"]))
            .unwrap()
            .unwrap();
        assert_eq!(fm.folders, strings(&["a", "{}"]));
        assert_eq!(fm.items, strings(&["b", "{parent}"]));
        assert_eq!(fm.properties, strings(&["b", "{parent}"]));
    }

    #[test]
    fn parse_args_splits_like_a_shell() {
        let fm = parse_args(args(&["--folders", "ghostty -e 'my fm' --select={}"]))
            .unwrap()
            .unwrap();
        assert_eq!(
            fm.folders,
            strings(&["ghostty", "-e", "my fm", "--select={}"])
        );
    }

    #[test]
    fn parse_args_rejects_bad_input() {
        for bad in [
            &["--folders", "echo"][..],
            &["--folders", ""],
            &["--folders", "echo '{}"],
            &["--folders", "echo {}", "--items", "echo"],
            &["--items", "echo {}"],
            &["--folders", "echo {}", "extra"],
            &["--folders"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn parse_args_help() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), None);
    }

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

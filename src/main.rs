use std::ffi::OsString;
use std::path::{Path, PathBuf};
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
        for argv in expand(template, &paths) {
            eprintln!("{method}: would run {argv:?} startup_id={startup_id:?}");
        }
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

/// Fills `template` with `paths` and returns the commands to run.
///
/// An argument that is exactly `{}` or `{parent}` becomes every path (or every distinct
/// parent) and the command runs once. A placeholder inside a longer argument, such as
/// `--select={}`, can only hold one path, so the command runs once per path instead.
fn expand(template: &[String], paths: &[PathBuf]) -> Vec<Vec<OsString>> {
    let per_path = template.iter().any(|arg| {
        (arg.contains("{}") || arg.contains("{parent}")) && arg != "{}" && arg != "{parent}"
    });
    let groups: Vec<&[PathBuf]> = match (paths.is_empty(), per_path) {
        (true, _) => Vec::new(),
        (false, true) => paths.chunks(1).collect(),
        (false, false) => vec![paths],
    };

    let mut commands = Vec::new();
    for group in groups {
        let mut parents: Vec<&Path> = Vec::new();
        for path in group {
            if !parents.contains(&parent(path)) {
                parents.push(parent(path));
            }
        }
        let mut argv = Vec::new();
        for arg in template {
            match arg.as_str() {
                "{}" => argv.extend(group.iter().map(|p| p.as_os_str().to_owned())),
                "{parent}" => argv.extend(parents.iter().map(|p| p.as_os_str().to_owned())),
                _ => argv.push(substitute(arg, &group[0])),
            }
        }
        // Several items in one folder give the same {parent} command; open it once.
        if !commands.contains(&argv) {
            commands.push(argv);
        }
    }
    commands
}

/// The folder holding `path`; the root is its own parent.
fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(path)
}

/// Replaces every `{}` and `{parent}` inside `arg`, keeping non-UTF-8 paths intact.
fn substitute(arg: &str, path: &Path) -> OsString {
    let mut out = OsString::new();
    let mut rest = arg;
    loop {
        let next = [("{}", path), ("{parent}", parent(path))]
            .into_iter()
            .filter_map(|(placeholder, value)| Some((rest.find(placeholder)?, placeholder, value)))
            .min_by_key(|&(i, ..)| i);
        let Some((i, placeholder, value)) = next else {
            break;
        };
        out.push(&rest[..i]);
        out.push(value);
        rest = &rest[i + placeholder.len()..];
    }
    out.push(rest);
    out
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

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn commands(commands: &[&[&str]]) -> Vec<Vec<OsString>> {
        commands.iter().map(|c| args(c)).collect()
    }

    #[test]
    fn expand_whole_argument_takes_all_paths_in_one_command() {
        let got = expand(&strings(&["yazi", "{}"]), &paths(&["/a/x", "/b/y"]));
        assert_eq!(got, commands(&[&["yazi", "/a/x", "/b/y"]]));
    }

    #[test]
    fn expand_embedded_placeholder_runs_once_per_path() {
        let got = expand(
            &strings(&["ranger", "--selectfile={}"]),
            &paths(&["/a/x", "/b/y"]),
        );
        let want = commands(&[
            &["ranger", "--selectfile=/a/x"],
            &["ranger", "--selectfile=/b/y"],
        ]);
        assert_eq!(got, want);
    }

    #[test]
    fn expand_dedups_parents() {
        let files = paths(&["/a/x", "/a/y", "/b/z"]);
        let got = expand(&strings(&["thunar", "{parent}"]), &files);
        assert_eq!(got, commands(&[&["thunar", "/a", "/b"]]));

        let got = expand(&strings(&["fm", "--dir={parent}"]), &files);
        assert_eq!(got, commands(&[&["fm", "--dir=/a"], &["fm", "--dir=/b"]]));
    }

    #[test]
    fn expand_substitutes_every_placeholder_in_an_argument() {
        let got = expand(&strings(&["fm", "{parent}:{}:{}"]), &paths(&["/a/x"]));
        assert_eq!(got, commands(&[&["fm", "/a:/a/x:/a/x"]]));
    }

    #[test]
    fn expand_keeps_each_path_one_argument() {
        let files = paths(&["/tmp/a b", "-rf"]);
        let got = expand(&strings(&["fm", "--", "{}"]), &files);
        assert_eq!(got, commands(&[&["fm", "--", "/tmp/a b", "-rf"]]));
    }

    #[test]
    fn expand_root_is_its_own_parent() {
        let got = expand(&strings(&["fm", "{parent}"]), &paths(&["/"]));
        assert_eq!(got, commands(&[&["fm", "/"]]));
    }

    #[test]
    fn expand_keeps_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt;

        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/a/\xff"));
        let got = expand(&strings(&["fm", "--select={}"]), &[path]);
        assert_eq!(got[0][1].as_bytes(), b"--select=/a/\xff");
    }

    #[test]
    fn expand_nothing_without_paths() {
        assert!(expand(&strings(&["fm", "{}"]), &[]).is_empty());
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

//! Called by both context menus: add-to-path.exe <add|remove> <path>...

#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};

use add_to_path_ext::path_store::{AddOutcome, PathStore, RemoveOutcome, broadcast_change, target_dir};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE, MessageBoxW};
use windows::core::{HSTRING, w};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let action = args.next().and_then(|a| a.into_string().ok()).unwrap_or_default();
    let paths: Vec<PathBuf> = args.map(|arg| clean_arg(&arg.to_string_lossy())).collect();

    if !matches!(action.as_str(), "add" | "remove") || paths.is_empty() {
        show("Usage: add-to-path.exe <add|remove> <path>...", MB_ICONERROR);
        return;
    }

    let store = PathStore::user();
    let mut changed = false;
    let mut failed = false;
    let mut lines = Vec::new();
    for path in &paths {
        let (line, did_change) = match apply(&store, &action, path) {
            Ok(result) => result,
            Err(message) => {
                failed = true;
                (message, false)
            }
        };
        changed |= did_change;
        lines.push(line);
    }

    if changed {
        broadcast_change();
        lines.push(String::new());
        lines.push("Terminals that are already open need restarting to see this.".to_owned());
    }
    show(&lines.join("\n"), if failed { MB_ICONERROR } else { MB_ICONINFORMATION });
}

fn apply(store: &PathStore, action: &str, path: &Path) -> Result<(String, bool), String> {
    let dir = target_dir(path)?;
    let shown = dir.display();
    if action == "add" {
        Ok(match store.add(&dir)? {
            AddOutcome::Added => (format!("Added to user PATH:\n{shown}"), true),
            AddOutcome::AlreadyInUserPath => (format!("Already in user PATH:\n{shown}"), false),
            AddOutcome::AlreadyInSystemPath => (format!("Already in system PATH, so nothing was added:\n{shown}"), false),
        })
    } else {
        Ok(match store.remove(&dir)? {
            RemoveOutcome::Removed => (format!("Removed from user PATH:\n{shown}"), true),
            RemoveOutcome::NotInUserPath => (format!("Not in user PATH:\n{shown}"), false),
        })
    }
}

/// Explorer passes a drive root as "D:\", and the backslash escapes the closing
/// quote during argument parsing, which leaves D:" behind. The backslash has to
/// come back, because a bare D: means the current folder on that drive.
fn clean_arg(arg: &str) -> PathBuf {
    match arg.strip_suffix('"') {
        Some(path) => PathBuf::from(format!("{path}\\")),
        None => PathBuf::from(arg),
    }
}

fn show(message: &str, icon: MESSAGEBOX_STYLE) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(message), w!("Add to PATH"), MB_OK | icon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_a_drive_root_mangled_by_argument_parsing() {
        assert_eq!(clean_arg("D:\""), PathBuf::from(r"D:\"));
    }

    #[test]
    fn leaves_ordinary_paths_alone() {
        assert_eq!(clean_arg(r"D:\Tools"), PathBuf::from(r"D:\Tools"));
    }
}

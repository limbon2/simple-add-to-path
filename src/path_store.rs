//! Reads and edits the PATH value in the registry.
//!
//! Existing entries are kept byte for byte, including unexpanded `%VARS%`, because
//! expanding them on write would quietly damage the user's PATH. Any read failure
//! other than "value not found" aborts the edit for the same reason.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, LPARAM, WPARAM};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE,
    REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE};
use windows::core::{HSTRING, PCWSTR, w};

const VALUE_NAME: PCWSTR = w!("Path");
const SYSTEM_ENV_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
/// Windows cannot hand a process an environment variable longer than this.
const MAX_ENV_VAR_CHARS: usize = 32_766;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AddOutcome {
    Added,
    AlreadyInUserPath,
    AlreadyInSystemPath,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RemoveOutcome {
    Removed,
    NotInUserPath,
}

/// The PATH value under a registry key in HKCU.
pub struct PathStore {
    subkey: String,
}

impl PathStore {
    /// The real user PATH.
    pub fn user() -> Self {
        Self::at(r"Environment")
    }

    /// A PATH value under any HKCU subkey, which lets tests avoid the real one.
    pub fn at(subkey: &str) -> Self {
        Self { subkey: subkey.to_owned() }
    }

    pub fn contains(&self, dir: &Path) -> Result<bool, String> {
        let raw = read_value(HKEY_CURRENT_USER, &self.subkey)?.map(|(raw, _)| raw).unwrap_or_default();
        Ok(raw_contains(&raw, dir))
    }

    pub fn add(&self, dir: &Path) -> Result<AddOutcome, String> {
        let text = dir.to_str().ok_or("The folder name contains invalid characters.")?;
        // A semicolon would split the folder into two PATH entries.
        if text.contains(';') {
            return Err("The folder name contains a semicolon, which PATH cannot hold safely.".to_owned());
        }

        let current = read_value(HKEY_CURRENT_USER, &self.subkey)?;
        let (raw, kind) = current.unwrap_or((String::new(), REG_EXPAND_SZ));
        if raw_contains(&raw, dir) {
            return Ok(AddOutcome::AlreadyInUserPath);
        }
        if let Some((system, _)) = read_value(HKEY_LOCAL_MACHINE, SYSTEM_ENV_KEY)? {
            if raw_contains(&system, dir) {
                return Ok(AddOutcome::AlreadyInSystemPath);
            }
        }

        let updated = append_entry(&raw, text);
        if updated.encode_utf16().count() > MAX_ENV_VAR_CHARS {
            return Err("Your user PATH is full: adding this folder would exceed the Windows limit.".to_owned());
        }
        write_value(&self.subkey, &updated, kind)?;
        Ok(AddOutcome::Added)
    }

    pub fn remove(&self, dir: &Path) -> Result<RemoveOutcome, String> {
        let Some((raw, kind)) = read_value(HKEY_CURRENT_USER, &self.subkey)? else {
            return Ok(RemoveOutcome::NotInUserPath);
        };
        match remove_entry(&raw, dir) {
            Some(updated) => {
                write_value(&self.subkey, &updated, kind)?;
                Ok(RemoveOutcome::Removed)
            }
            None => Ok(RemoveOutcome::NotInUserPath),
        }
    }
}

/// Turns the selected item into the folder PATH needs: an app becomes its parent folder.
pub fn target_dir(selected: &Path) -> Result<PathBuf, String> {
    let meta = std::fs::metadata(selected).map_err(|e| format!("Cannot read {}: {e}", selected.display()))?;
    if meta.is_dir() {
        return Ok(selected.to_path_buf());
    }
    selected
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no parent folder.", selected.display()))
}

/// Tells running programs, including Explorer, that the environment changed.
pub fn broadcast_change() {
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}

fn raw_contains(raw: &str, dir: &Path) -> bool {
    let wanted = normalize(&dir.to_string_lossy());
    split_entries(raw).iter().any(|entry| normalize(entry) == wanted)
}

fn append_entry(raw: &str, entry: &str) -> String {
    let kept = raw.trim_end_matches(';');
    if kept.is_empty() { entry.to_owned() } else { format!("{kept};{entry}") }
}

/// Returns the new value, or `None` when no entry matches.
fn remove_entry(raw: &str, dir: &Path) -> Option<String> {
    let wanted = normalize(&dir.to_string_lossy());
    let entries = split_entries(raw);
    let kept: Vec<&str> = entries.iter().copied().filter(|entry| normalize(entry) != wanted).collect();
    (kept.len() != entries.len()).then(|| kept.join(";"))
}

/// Splits on semicolons, except inside quotes: `"C:\a;b"` is one legal entry.
fn split_entries(raw: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                entries.push(&raw[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    entries.push(&raw[start..]);
    entries
}

/// A comparison key only, never written back. `C:\Tools\`, `c:/tools` and `"C:\Tools"` all match.
fn normalize(entry: &str) -> String {
    let expanded = expand_env(entry.trim().trim_matches('"'));
    let slashes = expanded.trim().replace('/', r"\");
    let trimmed = slashes.trim_end_matches('\\');
    // Keep the backslash on a drive root, since `C:` means something different from `C:\`.
    let key = if trimmed.len() == 2 && trimmed.ends_with(':') { format!(r"{trimmed}\") } else { trimmed.to_owned() };
    key.to_lowercase()
}

fn expand_env(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let source = HSTRING::from(text);
    unsafe {
        let needed = ExpandEnvironmentStringsW(&source, None);
        if needed == 0 {
            return text.to_owned();
        }
        let mut buffer = vec![0u16; needed as usize];
        let written = ExpandEnvironmentStringsW(&source, Some(&mut buffer));
        if written == 0 || written > needed {
            return text.to_owned();
        }
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    }
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn read_value(root: HKEY, subkey: &str) -> Result<Option<(String, REG_VALUE_TYPE)>, String> {
    let mut handle = HKEY::default();
    let opened = unsafe { RegOpenKeyExW(root, &HSTRING::from(subkey), None, KEY_QUERY_VALUE, &mut handle) };
    if opened == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if opened != ERROR_SUCCESS {
        return Err(format!("Cannot open the PATH registry key: {}", opened.to_hresult().message()));
    }
    let key = Key(handle);

    let mut buffer = vec![0u16; 8192];
    loop {
        let mut kind = REG_VALUE_TYPE::default();
        let mut bytes = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                VALUE_NAME,
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status == ERROR_MORE_DATA {
            buffer.resize((bytes as usize).div_ceil(2) + 1, 0);
            continue;
        }
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(format!("Cannot read PATH: {}", status.to_hresult().message()));
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err("PATH is stored in an unexpected format, so it was left alone.".to_owned());
        }
        let units = &buffer[..bytes as usize / 2];
        let end = units.iter().rposition(|&u| u != 0).map_or(0, |i| i + 1);
        let text = String::from_utf16(&units[..end]).map_err(|_| "PATH contains invalid text, so it was left alone.")?;
        return Ok(Some((text, kind)));
    }
}

fn write_value(subkey: &str, value: &str, kind: REG_VALUE_TYPE) -> Result<(), String> {
    let mut handle = HKEY::default();
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut handle,
            None,
        )
    };
    if created != ERROR_SUCCESS {
        return Err(format!("Cannot open the PATH registry key: {}", created.to_hresult().message()));
    }
    let key = Key(handle);

    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
    let status = unsafe { RegSetValueExW(key.0, VALUE_NAME, None, kind, Some(bytes)) };
    if status != ERROR_SUCCESS {
        return Err(format!("Cannot save PATH: {}", status.to_hresult().message()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW};

    /// A throwaway HKCU key, so tests never touch the real PATH.
    struct TestKey(String);

    impl TestKey {
        fn new(name: &str) -> Self {
            let key = Self(format!(r"Software\SimpleAddToPathTest\{name}"));
            key.delete();
            key
        }
        fn delete(&self) {
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(self.0.as_str()));
            }
        }
        fn store(&self) -> PathStore {
            PathStore::at(&self.0)
        }
        fn raw(&self) -> (String, REG_VALUE_TYPE) {
            read_value(HKEY_CURRENT_USER, &self.0).unwrap().unwrap()
        }
    }

    impl Drop for TestKey {
        fn drop(&mut self) {
            self.delete();
            // Fails harmlessly while other tests still have keys under the parent.
            unsafe {
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, w!(r"Software\SimpleAddToPathTest"));
            }
        }
    }

    #[test]
    fn creates_the_value_when_missing() {
        let key = TestKey::new("missing");
        assert_eq!(key.store().add(Path::new(r"Z:\sat\new")).unwrap(), AddOutcome::Added);
        assert_eq!(key.raw(), (r"Z:\sat\new".to_owned(), REG_EXPAND_SZ));
    }

    #[test]
    fn keeps_existing_entries_and_value_type_untouched() {
        let key = TestKey::new("preserve");
        write_value(&key.0, r"%USERPROFILE%\bin;;C:\Odd Dir\;", REG_EXPAND_SZ).unwrap();
        key.store().add(Path::new(r"Z:\sat\new")).unwrap();
        assert_eq!(key.raw(), (r"%USERPROFILE%\bin;;C:\Odd Dir\;Z:\sat\new".to_owned(), REG_EXPAND_SZ));
        key.store().remove(Path::new(r"Z:\sat\new")).unwrap();
        assert_eq!(key.raw().0, r"%USERPROFILE%\bin;;C:\Odd Dir\");
    }

    #[test]
    fn keeps_a_plain_string_value_as_plain() {
        let key = TestKey::new("plain");
        write_value(&key.0, r"C:\one", REG_SZ).unwrap();
        key.store().add(Path::new(r"Z:\sat\new")).unwrap();
        assert_eq!(key.raw().1, REG_SZ);
    }

    #[test]
    fn does_not_add_a_duplicate_in_another_spelling() {
        let key = TestKey::new("duplicate");
        write_value(&key.0, r#"Z:\Sat\Tools\;"Z:\quoted""#, REG_EXPAND_SZ).unwrap();
        let store = key.store();
        assert_eq!(store.add(Path::new(r"z:/sat/tools")).unwrap(), AddOutcome::AlreadyInUserPath);
        assert_eq!(store.add(Path::new(r"z:\QUOTED\")).unwrap(), AddOutcome::AlreadyInUserPath);
    }

    #[test]
    fn matches_entries_written_with_variables() {
        let key = TestKey::new("variables");
        write_value(&key.0, r"%SystemRoot%\sat-tools;C:\keep", REG_EXPAND_SZ).unwrap();
        let dir = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("sat-tools");
        assert!(key.store().contains(&dir).unwrap());
        assert_eq!(key.store().remove(&dir).unwrap(), RemoveOutcome::Removed);
        assert_eq!(key.raw().0, r"C:\keep");
    }

    #[test]
    fn does_not_match_a_prefix_of_another_entry() {
        let key = TestKey::new("prefix");
        write_value(&key.0, r"Z:\sat\tools-extra", REG_EXPAND_SZ).unwrap();
        let store = key.store();
        assert!(!store.contains(Path::new(r"Z:\sat\tools")).unwrap());
        assert_eq!(store.remove(Path::new(r"Z:\sat\tools")).unwrap(), RemoveOutcome::NotInUserPath);
        assert_eq!(key.raw().0, r"Z:\sat\tools-extra");
    }

    #[test]
    fn removes_every_copy_of_a_repeated_entry() {
        let key = TestKey::new("repeat");
        write_value(&key.0, r"Z:\a;C:\keep;z:\A\", REG_EXPAND_SZ).unwrap();
        key.store().remove(Path::new(r"Z:\a")).unwrap();
        assert_eq!(key.raw().0, r"C:\keep");
    }

    #[test]
    fn a_quoted_entry_with_a_semicolon_stays_whole() {
        let key = TestKey::new("quoted");
        write_value(&key.0, r#""C:\a;b";Z:\x"#, REG_EXPAND_SZ).unwrap();
        key.store().remove(Path::new(r"Z:\x")).unwrap();
        assert_eq!(key.raw().0, r#""C:\a;b""#);
    }

    #[test]
    fn refuses_a_folder_with_a_semicolon() {
        let key = TestKey::new("semicolon");
        assert!(key.store().add(Path::new(r"Z:\a;b")).is_err());
    }

    #[test]
    fn reports_a_folder_already_on_the_system_path() {
        let key = TestKey::new("system");
        let (system, _) = read_value(HKEY_LOCAL_MACHINE, SYSTEM_ENV_KEY).unwrap().unwrap();
        let first = split_entries(&system).into_iter().find(|e| !e.is_empty()).unwrap().to_owned();
        let dir = PathBuf::from(expand_env(&first));
        assert_eq!(key.store().add(&dir).unwrap(), AddOutcome::AlreadyInSystemPath);
    }

    #[test]
    fn rejects_a_path_too_long_for_windows() {
        let key = TestKey::new("full");
        write_value(&key.0, &"x".repeat(MAX_ENV_VAR_CHARS), REG_EXPAND_SZ).unwrap();
        assert!(key.store().add(Path::new(r"Z:\sat\new")).is_err());
        assert_eq!(key.raw().0.len(), MAX_ENV_VAR_CHARS);
    }

    #[test]
    fn reads_a_value_larger_than_the_first_buffer() {
        let key = TestKey::new("large");
        let long = format!("{};C:\\end", "y".repeat(20_000));
        write_value(&key.0, &long, REG_EXPAND_SZ).unwrap();
        assert_eq!(key.raw().0, long);
    }

    #[test]
    fn an_app_resolves_to_its_folder() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(target_dir(&exe).unwrap(), exe.parent().unwrap());
        let dir = exe.parent().unwrap();
        assert_eq!(target_dir(dir).unwrap(), dir);
        assert!(target_dir(Path::new(r"Z:\does\not\exist")).is_err());
    }

    #[test]
    fn drive_roots_compare_correctly() {
        assert_eq!(normalize(r"D:\"), normalize("d:/"));
        assert_ne!(normalize(r"D:\"), normalize(r"D:\x"));
    }
}

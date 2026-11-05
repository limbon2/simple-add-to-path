//! The entry in the top-level Windows 11 menu. It picks the label and hands the
//! actual PATH edit to add-to-path.exe, which sits next to this DLL.

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER, HINSTANCE, HMODULE, MAX_PATH, S_FALSE,
};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray,
    SHStrDupW, SIGDN_FILESYSPATH,
};
use windows::core::{BOOL, Error, GUID, HRESULT, HSTRING, IUnknown, Interface, PWSTR, Ref, Result, implement, w};

use crate::path_store::{PathStore, target_dir};

/// Must match `Clsid` and `com:Class Id` in packaging/AppxManifest.xml.
const CLSID_ADD_TO_PATH: GUID = GUID::from_u128(0x7d1f2a64_3b9e_4c58_a1d7_5e0c92b4f613);

static MODULE: AtomicIsize = AtomicIsize::new(0);

#[unsafe(no_mangle)]
extern "system" fn DllMain(module: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module.0 as isize, Ordering::Relaxed);
    }
    true.into()
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe {
        *out = std::ptr::null_mut();
        if *clsid != CLSID_ADD_TO_PATH {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, out)
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let command: IExplorerCommand = AddToPathCommand.into();
        unsafe { command.query(iid, out).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[implement(IExplorerCommand)]
struct AddToPathCommand;

impl IExplorerCommand_Impl for AddToPathCommand_Impl {
    fn GetTitle(&self, items: Ref<IShellItemArray>) -> Result<PWSTR> {
        let title = match selection_state(&selected_paths(items.as_ref())) {
            Selection::AllInPath => w!("Remove from user PATH"),
            Selection::SomeMissing => w!("Add to user PATH"),
            // Explorer may ask for the label without saying what was clicked. The
            // label has to stay true whichever way the click ends up going.
            Selection::Unknown => w!("Add to / remove from user PATH"),
        };
        unsafe { SHStrDupW(title) }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        let mut buffer = [0u16; MAX_PATH as usize];
        let source = w!(r"%SystemRoot%\System32\SystemPropertiesAdvanced.exe,0");
        let written = unsafe { ExpandEnvironmentStringsW(source, Some(&mut buffer)) };
        if written == 0 || written as usize > buffer.len() {
            return Err(Error::from_thread());
        }
        unsafe { SHStrDupW(&HSTRING::from_wide(&buffer[..written as usize - 1])) }
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(CLSID_ADD_TO_PATH)
    }

    fn GetState(&self, items: Ref<IShellItemArray>, _ok_to_be_slow: BOOL) -> Result<u32> {
        // Items outside the file system, such as This PC, have no folder to add.
        let has_virtual_item = items.as_ref().is_some_and(|array| {
            let count = unsafe { array.GetCount() }.unwrap_or(0);
            selected_paths(Some(array)).len() != count as usize
        });
        Ok(if has_virtual_item { ECS_HIDDEN.0 as u32 } else { ECS_ENABLED.0 as u32 })
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _context: Ref<IBindCtx>) -> Result<()> {
        let paths = selected_paths(items.as_ref());
        if paths.is_empty() {
            return Ok(());
        }
        let action = match selection_state(&paths) {
            Selection::AllInPath => "remove",
            _ => "add",
        };
        Command::new(helper_exe()?)
            .arg(action)
            .args(&paths)
            .spawn()
            .map_err(|e| Error::new(E_FAIL, e.to_string()))?;
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

enum Selection {
    AllInPath,
    SomeMissing,
    Unknown,
}

fn selection_state(paths: &[PathBuf]) -> Selection {
    if paths.is_empty() {
        return Selection::Unknown;
    }
    let store = PathStore::user();
    let all_present = paths.iter().all(|path| {
        target_dir(path).and_then(|dir| store.contains(&dir)).unwrap_or(false)
    });
    if all_present { Selection::AllInPath } else { Selection::SomeMissing }
}

fn selected_paths(items: Option<&IShellItemArray>) -> Vec<PathBuf> {
    let Some(array) = items else {
        return Vec::new();
    };
    let count = unsafe { array.GetCount() }.unwrap_or(0);
    (0..count)
        .filter_map(|index| unsafe {
            let item = array.GetItemAt(index).ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let text = name.to_string();
            CoTaskMemFree(Some(name.0 as *const c_void));
            text.ok().map(PathBuf::from)
        })
        .collect()
}

fn helper_exe() -> Result<PathBuf> {
    let module = HMODULE(MODULE.load(Ordering::Relaxed) as *mut c_void);
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    if length == 0 {
        return Err(Error::from_thread());
    }
    let dll = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    Ok(dll.with_file_name("add-to-path.exe"))
}

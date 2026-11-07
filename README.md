# simple-add-to-path

Adds an **Add to user PATH** option to the Windows File Explorer right-click menu.

Right-click a folder, the empty space inside a folder, or an `.exe`, `.bat` or `.cmd` file. For a file, the folder containing it is added, since PATH only holds folders.

| Menu | Adding | Removing |
|---|---|---|
| Windows 11 top-level menu | **Add to user PATH** | The label changes to **Remove from user PATH** once the folder is in PATH |
| Windows 10 and Windows 11 "Show more options" | **Add to user PATH** | Shift + right-click, then **Remove from user PATH** |

A dialog confirms each change. Terminals that are already open need restarting to see the new PATH.

Only the user PATH is changed, so no admin rights are needed. Existing entries are kept exactly as written, including `%VARIABLES%`.

## Requirements

- Windows 10 version 2004 or later, or Windows 11, on x64
- [Rust](https://rustup.rs/) with the `x86_64-pc-windows-msvc` toolchain, which needs the Visual Studio C++ build tools
- For the Windows 11 top-level menu only: Developer Mode, under **Settings > System > For developers**

Without Developer Mode, the install script stops with an error at the Windows 11 step. The "Show more options" entries are already in place by then and keep working.

## Install

```bash
git clone https://github.com/limbon2/simple-add-to-path.git
```

```bash
cd simple-add-to-path
```

```bash
powershell -ExecutionPolicy Bypass -File install.ps1
```

The script builds the project, copies it to `%LOCALAPPDATA%\Programs\AddToPath` and registers the menu entries for the current user. Run it again to update.

If the entry doesn't appear in the Windows 11 top-level menu, restart Explorer or sign out and back in.

## Uninstall

```bash
powershell -ExecutionPolicy Bypass -File uninstall.ps1
```

This removes the menu entries and installed files. Your PATH is left as it is, so remove any folders you no longer want first.

## Command line

The helper behind the menus can also be run directly, with one or more paths:

```bash
"%LOCALAPPDATA%\Programs\AddToPath\add-to-path.exe" add "C:\Tools"
```

```bash
"%LOCALAPPDATA%\Programs\AddToPath\add-to-path.exe" remove "C:\Tools"
```

## Limitations

- Folders with a semicolon in their name can't be added.
- A folder already in the system PATH isn't added again, and the system PATH can't be edited from here.
- Right-clicking a drive itself in **This PC** shows no entry. Right-click the empty space inside the drive instead.
- It has been developed on Windows 11. The Windows 10 menu uses the same registry entries as "Show more options", but hasn't been tested on Windows 10 itself.

## Development

```bash
cargo test
```

The tests use a temporary registry key under `HKEY_CURRENT_USER\Software`, so they don't touch your real PATH.

| Path | Purpose |
|---|---|
| `src/path_store.rs` | Reads and edits PATH in the registry |
| `src/main.rs` | The `add-to-path.exe` helper that both menus call |
| `src/explorer_command.rs` | The Windows 11 menu command, built as a DLL |
| `packaging/AppxManifest.xml` | Registers that command with Windows 11 |
| `install.ps1`, `uninstall.ps1`, `common.ps1` | Install and uninstall scripts |

## License

[MIT](LICENSE)

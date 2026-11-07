use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/connected.ico");
    println!("cargo:rerun-if-changed=assets/website-dialog.rc");
    println!("cargo:rerun-if-changed=assets/help-dialog.rc");
    println!("cargo:rerun-if-changed=assets/hotkey-dialog.rc");
    println!("cargo:rerun-if-changed=assets/speed-dialog.rc");
    println!("cargo:rerun-if-changed=assets/network-tools-dialog.rc");
    println!("cargo:rerun-if-changed=assets/status-dialog.rc");
    println!("cargo:rerun-if-changed=assets/power-dialog.rc");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    if !env::var("TARGET")
        .unwrap_or_default()
        .ends_with("windows-gnu")
    {
        return;
    }
    // Keep windres inputs in the ASCII Cargo target directory (MinGW may not
    // handle the Chinese source path consistently).
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("assets/icons/connected.ico", output.join("app.ico")).expect("copy application icon");
    fs::copy("assets/app.manifest", output.join("app.manifest")).expect("copy manifest");
    let dialog =
        fs::read_to_string("assets/website-dialog.rc").expect("read website dialog resource");
    let help = fs::read_to_string("assets/help-dialog.rc").expect("read help dialog resource");
    let hotkeys =
        fs::read_to_string("assets/hotkey-dialog.rc").expect("read hotkey dialog resource");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let speed = fs::read_to_string("assets/speed-dialog.rc").expect("read speed dialog resource");
    let network_tools = fs::read_to_string("assets/network-tools-dialog.rc").expect("read network tools resources");
    let status_dialog = fs::read_to_string("assets/status-dialog.rc").expect("read status resource");
    let power_dialog = fs::read_to_string("assets/power-dialog.rc").expect("read power resource");
    let numeric_version = format!("{},0", version.replace('.', ","));
    let version_resource = format!(
        r#"
#include <winver.h>
1 VERSIONINFO
 FILEVERSION {numeric_version}
 PRODUCTVERSION {numeric_version}
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0x0L
 FILEOS VOS_NT_WINDOWS32
 FILETYPE VFT_APP
 FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "SHUNet\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "InternalName", "SHUNet\0"
            VALUE "OriginalFilename", "SHUNet.exe\0"
            VALUE "ProductName", "SHUNet\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
    );
    fs::write(
        output.join("tray.rc"),
        format!("1 ICON \"app.ico\"\n1 24 \"app.manifest\"\n{dialog}\n{help}\n{hotkeys}\n{speed}\n{network_tools}\n{status_dialog}\n{power_dialog}\n{version_resource}"),
    )
    .expect("write resource source");
    let status = Command::new("windres")
        .current_dir(&output)
        .args(["-i", "tray.rc", "-o", "tray-icon.o", "-O", "coff"])
        .status()
        .expect("MinGW windres is required (add mingw64/bin to PATH)");
    assert!(status.success(), "compile application resources");
    println!(
        "cargo:rustc-link-arg={}",
        output.join("tray-icon.o").display()
    );
}

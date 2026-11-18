//! vork-setup.exe: installs vØrk storage for the current user, no administrator
//! needed, and uninstalls it (`--uninstall`, from Apps & features).
//!
//! - The programs go to `%LOCALAPPDATA%\Programs\vork`: vorkd, the
//!   vØrk storage command, the Cloud Files provider and vork-desktop, the
//!   utility in the notification area. No Reticulum: that's Prns's own
//!   (or another shared instance), installed apart.
//! - vork-desktop starts at login (`--autostart`) and starts vorkd once
//!   Reticulum is up; nothing runs as a service.
//! - The folder is put on the user's PATH (for `vork`), a Start menu
//!   shortcut is made, and Apps & features lists it.
//! - Uninstalling removes all that and leaves the user's data: the store in
//!   `%LOCALAPPDATA%\vork` and Reticulum's `%USERPROFILE%\.reticulum`.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod setup;

#[cfg(windows)]
fn main() {
    setup::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("vork-setup is the Windows installer");
}

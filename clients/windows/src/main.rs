//! vork-desktop for Windows: native Win32 controls over the shared core,
//! and an icon in the notification area. It's also what starts vØrk storage at
//! login (`--autostart`): the vØrk node where none runs yet, then vorkd.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod libraries;
#[cfg(windows)]
mod recovery;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("vork-desktop-win is the Windows front end; on Linux use vork-desktop-gtk");
}

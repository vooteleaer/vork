//! vork-setup.exe: installs vØrk the network for the whole computer, and
//! uninstalls it (`--uninstall`, from Apps & features). Storage isn't part of it.
//!
//! - It asks for an administrator (UAC): the node runs as a Windows service
//!   and makes a network interface (wintun).
//! - The programs go to `%ProgramFiles%\vork`: the node (`vork.exe`),
//!   `wintun.dll` next to it, and the window with its tray icon (`vork-net.exe`).
//! - A new node gets its configuration and identity in `%ProgramData%\vork`,
//!   with the interface on; an existing one is kept as it is.
//! - The node is the `vork` service (started at boot), let through the
//!   firewall; vork-net starts in the tray at every login; the folder is on
//!   the PATH (for `vork`); the Start menu and Apps & features list it.
//! - An upgrade stops the window and the service, replaces the programs and
//!   starts them again.
//! - Uninstalling removes all that and keeps `%ProgramData%\vork`.
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

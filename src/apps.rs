//! Pages that applications on this machine add to the node's dashboard.
//!
//! An application connected to the local API says which pages it has
//! (`PAGES id title {path title}…`); the dashboard lists them in its menu and
//! asks the application for their content through the node (`PAGE`, answered
//! with `RESPOND`). The node knows nothing about any one application: it
//! keeps what each said in `<state>/apps.json`, so the menu stays the same
//! across restarts and says "stopped" while the application isn't connected.
//! An application that says it has no pages is forgotten; so is one the
//! owner removes from the dashboard while it's stopped.

use std::path::{Path, PathBuf};

/// Pages one application may add.
pub const MAX_PAGES: usize = 8;
/// Applications with pages, at most.
pub const MAX_APPS: usize = 16;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Page {
    pub path: String,
    pub title: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct App {
    pub id: String,
    pub title: String,
    pub pages: Vec<Page>,
    /// The API client serving it now (not kept).
    #[serde(skip)]
    pub client: Option<u32>,
}

pub struct Apps {
    file: Option<PathBuf>,
    pub list: Vec<App>,
}

/// An id is short, lower-case letters, digits and dashes: it's in the
/// dashboard's addresses and shown to the owner as where a page comes from.
pub fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A page's path within its application: `/`-led, no spaces or controls.
pub fn valid_path(p: &str) -> bool {
    p.starts_with('/') && p.len() <= 128 && p.bytes().all(|b| b.is_ascii_graphic())
}

fn clean_title(t: &str) -> String {
    t.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string()
}

impl Apps {
    pub fn open(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join("apps.json"));
        let list = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<Vec<App>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|a| valid_id(&a.id))
            .take(MAX_APPS)
            .collect();
        Apps { file, list }
    }

    fn save(&self) {
        if let Some(f) = &self.file {
            if let Ok(b) = serde_json::to_vec_pretty(&self.list) {
                let tmp = f.with_extension("json.tmp");
                if std::fs::write(&tmp, b).is_ok() {
                    let _ = std::fs::rename(&tmp, f);
                }
            }
        }
    }

    /// What a client said: its pages (none: forget the application).
    /// Returns an error to tell the client, or whether the menu changed.
    pub fn set(&mut self, client: u32, id: &str, title: &str, pages: Vec<(String, String)>) -> Result<bool, String> {
        if !valid_id(id) {
            return Err(format!("not an application id: {id:?} (a-z, 0-9, -; at most 32)"));
        }
        if let Some(a) = self.list.iter().find(|a| a.id == id) {
            if a.client.is_some_and(|c| c != client) {
                return Err(format!("{id} is served by another connection"));
            }
        }
        if pages.is_empty() {
            let before = self.list.len();
            self.list.retain(|a| a.id != id);
            let changed = self.list.len() != before;
            if changed {
                self.save();
            }
            return Ok(changed);
        }
        let mut seen = Vec::new();
        let mut list = Vec::new();
        for (path, t) in pages.into_iter().take(MAX_PAGES) {
            if !valid_path(&path) {
                return Err(format!("not a page path: {path:?}"));
            }
            if !seen.contains(&path) {
                seen.push(path.clone());
                let t = clean_title(&t);
                list.push(Page { title: if t.is_empty() { path.clone() } else { t }, path });
            }
        }
        let title = match clean_title(title) {
            t if t.is_empty() => id.to_string(),
            t => t,
        };
        let full = self.list.len() >= MAX_APPS;
        let changed = match self.list.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                let changed = a.title != title || a.pages != list;
                a.title = title;
                a.pages = list;
                a.client = Some(client);
                changed
            }
            None if full => return Err("too many applications with pages".into()),
            None => {
                self.list.push(App { id: id.to_string(), title, pages: list, client: Some(client) });
                true
            }
        };
        if changed {
            self.save();
        }
        Ok(changed)
    }

    /// A client went away: its applications are stopped (still in the menu).
    pub fn gone(&mut self, client: u32) {
        for a in self.list.iter_mut().filter(|a| a.client == Some(client)) {
            a.client = None;
        }
    }

    /// Removed by the owner: only a stopped one (a running one would say it again).
    pub fn forget(&mut self, id: &str) -> Result<(), String> {
        match self.list.iter().position(|a| a.id == id) {
            None => Err(format!("no application {id}")),
            Some(i) if self.list[i].client.is_some() => Err(format!("{id} is running: stop it first")),
            Some(i) => {
                self.list.remove(i);
                self.save();
                Ok(())
            }
        }
    }

    pub fn serving(&self, id: &str) -> Option<u32> {
        self.list.iter().find(|a| a.id == id).and_then(|a| a.client)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.list
                .iter()
                .map(|a| serde_json::json!({ "id": a.id, "title": a.title, "pages": a.pages, "running": a.client.is_some() }))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn kept_across_restarts_and_stopped_until_served() {
        let dir = std::env::temp_dir().join(format!("vork-apps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = Apps::open(Some(&dir));
        assert_eq!(a.set(1, "store", "Storage", pages(&[("/", "Overview"), ("/libraries", "Libraries")])), Ok(true));
        assert_eq!(a.set(1, "store", "Storage", pages(&[("/", "Overview"), ("/libraries", "Libraries")])), Ok(false));
        assert!(a.set(2, "store", "Other", pages(&[("/", "x")])).is_err(), "another connection can't take it over");
        let b = Apps::open(Some(&dir));
        assert_eq!(b.list.len(), 1);
        assert_eq!(b.serving("store"), None);
        assert_eq!(b.list[0].pages[1].title, "Libraries");
        a.gone(1);
        assert_eq!(a.serving("store"), None);
        assert_eq!(a.set(2, "store", "Storage", vec![]), Ok(true), "no pages: forgotten");
        assert!(Apps::open(Some(&dir)).list.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_and_paths_checked() {
        let mut a = Apps::open(None);
        assert!(a.set(1, "Bad Id", "x", pages(&[("/", "x")])).is_err());
        assert!(a.set(1, "ok", "x", pages(&[("no-slash", "x")])).is_err());
        assert!(a.set(1, "ok", "x", pages(&[("/a b", "x")])).is_err());
        assert_eq!(a.set(1, "ok", "\u{7}  ", pages(&[("/", "")])), Ok(true));
        assert_eq!(a.list[0].title, "ok");
        assert_eq!(a.list[0].pages[0].title, "/");
        assert!(a.forget("ok").is_err(), "running");
        a.gone(1);
        assert!(a.forget("ok").is_ok());
    }
}

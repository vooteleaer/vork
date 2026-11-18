//! Saved queries (ADR-0020): records in the user's own library `me`, so
//! every device of theirs has them. `type = retifs.query`, `name`, `query`
//! (terms as `vork query` takes them, comma-separated) and `scope`: the
//! libraries it looks in, by their creators' keys (stable, and the same on
//! every device whatever each calls them); empty: every library.
//!
//! Favorite libraries are kept the same way: `type = retifs.favorite`,
//! `library` (its creator's key). So are members' nicknames:
//! `type = retifs.nickname`, `name`, `retifs:node` (the device), `retifs:writer`.
//!
//! One convention for the CLI, the desktop utility's Find and any app.

use std::collections::HashMap;

use crate::{AttrValue, Client, Request, Response};

/// The user's own library: contacts, saved queries, settings.
pub const ME: &str = "me";
pub const TYPE: &str = "retifs.query";

#[derive(Debug, Clone, PartialEq)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub query: String,
    /// Creator keys (hex); empty: every library.
    pub scope: Vec<String>,
}

impl SavedQuery {
    pub fn terms(&self) -> Vec<String> {
        split_terms(&self.query)
    }
}

/// `a=1, b^=x` → `["a=1", "b^=x"]`.
pub fn split_terms(query: &str) -> Vec<String> {
    query.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect()
}

fn err(r: Result<Response, crate::ClientError>) -> Result<Response, String> {
    match r {
        Ok(Response::Error { message }) => Err(message),
        Ok(r) => Ok(r),
        Err(e) => Err(e.to_string()),
    }
}

/// The saved queries, by name.
pub async fn list(c: &mut Client) -> Vec<SavedQuery> {
    let req = Request::Query { ns: ME.into(), terms: vec![format!("type={TYPE}")], at: None };
    let Ok(Response::Records { records }) = c.call(req).await else { return Vec::new() };
    let mut out: Vec<SavedQuery> = records
        .iter()
        .filter_map(|r| {
            let get = |k: &str| r.attrs.iter().find(|(key, _)| key == k).map(|(_, v)| v);
            let text = |k: &str| match get(k) {
                Some(AttrValue::Str(s)) => Some(s.clone()),
                _ => None,
            };
            let scope = match get("scope") {
                Some(AttrValue::List(items)) => items.iter().filter_map(|v| if let AttrValue::Str(s) = v { Some(s.clone()) } else { None }).collect(),
                _ => Vec::new(),
            };
            Some(SavedQuery { id: r.id.clone(), name: text("name")?, query: text("query")?, scope })
        })
        .collect();
    out.sort_by_key(|q| q.name.to_lowercase());
    out
}

/// Every library on this device but `me`.
pub async fn namespaces(c: &mut Client) -> Result<Vec<String>, String> {
    match err(c.call(Request::Namespaces).await)? {
        Response::Names { names } => Ok(names.into_iter().filter(|n| n != ME).collect()),
        _ => Ok(Vec::new()),
    }
}

/// A library's creator key (hex), from its address (`retifs-ch:1:<creator>:<read key>`).
pub async fn creator(c: &mut Client, ns: &str) -> Result<String, String> {
    match err(c.call(Request::NsInfo { ns: ns.into() }).await)? {
        Response::Namespace(v) => v.address.split(':').nth(2).map(String::from).ok_or_else(|| "a malformed address".into()),
        _ => Err("unexpected answer".into()),
    }
}

/// This device's libraries by creator key.
pub async fn by_creator(c: &mut Client) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for n in namespaces(c).await? {
        if let Ok(w) = creator(c, &n).await {
            out.insert(w, n);
        }
    }
    Ok(out)
}

/// Where a saved query looks on this device: its libraries' local names,
/// and the creator keys of those not here.
pub async fn resolve(c: &mut Client, q: &SavedQuery) -> Result<(Vec<String>, Vec<String>), String> {
    if q.scope.is_empty() {
        return Ok((namespaces(c).await?, Vec::new()));
    }
    let by = by_creator(c).await?;
    let (mut here, mut missing) = (Vec::new(), Vec::new());
    for w in &q.scope {
        match by.get(w) {
            Some(n) => here.push(n.clone()),
            None => missing.push(w.clone()),
        }
    }
    Ok((here, missing))
}

/// Saves (or replaces, by name) `terms` over libraries `scope` (local
/// names; empty: every library).
pub async fn save(c: &mut Client, name: &str, scope: &[String], terms: &[String]) -> Result<(), String> {
    let mut keys = Vec::new();
    for n in scope {
        keys.push(AttrValue::Str(creator(c, n).await?));
    }
    let attrs = vec![
        ("name".to_string(), AttrValue::Str(name.into())),
        ("query".to_string(), AttrValue::Str(terms.join(", "))),
        ("scope".to_string(), AttrValue::List(keys)),
    ];
    let req = match list(c).await.into_iter().find(|q| q.name.eq_ignore_ascii_case(name)) {
        Some(q) => Request::RecordSet { ns: ME.into(), id: q.id, attrs, unset: vec![], data: None },
        None => {
            let mut attrs = attrs;
            attrs.push(("type".into(), AttrValue::Str(TYPE.into())));
            Request::RecordCreate { ns: ME.into(), attrs, data: None }
        }
    };
    err(c.call(req).await).map(|_| ())
}

/// Removes saved query `name`.
pub async fn remove(c: &mut Client, name: &str) -> Result<(), String> {
    let q = list(c).await.into_iter().find(|q| q.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("no saved query {name}"))?;
    err(c.call(Request::RecordDelete { ns: ME.into(), id: q.id }).await).map(|_| ())
}

/// A favorite library's record type (in `me`, so every device of the
/// user's has it): `library`, its creator key; apps list favorites first.
pub const FAVORITE_TYPE: &str = "retifs.favorite";

/// The favorite libraries: their creator keys, with their records' IDs.
pub async fn favorites(c: &mut Client) -> Vec<(String, String)> {
    let req = Request::Query { ns: ME.into(), terms: vec![format!("type={FAVORITE_TYPE}")], at: None };
    let Ok(Response::Records { records }) = c.call(req).await else { return Vec::new() };
    records
        .into_iter()
        .filter_map(|r| match r.attrs.iter().find(|(k, _)| k == "library").map(|(_, v)| v) {
            Some(AttrValue::Str(w)) => Some((w.clone(), r.id.clone())),
            _ => None,
        })
        .collect()
}

/// Makes library `ns` (its local name) a favorite, or not.
pub async fn set_favorite(c: &mut Client, ns: &str, on: bool) -> Result<(), String> {
    let w = creator(c, ns).await?;
    let held: Vec<String> = favorites(c).await.into_iter().filter(|(k, _)| *k == w).map(|(_, id)| id).collect();
    if on && held.is_empty() {
        let attrs = vec![("type".to_string(), AttrValue::Str(FAVORITE_TYPE.into())), ("library".to_string(), AttrValue::Str(w))];
        err(c.call(Request::RecordCreate { ns: ME.into(), attrs, data: None }).await)?;
    } else if !on {
        for id in held {
            err(c.call(Request::RecordDelete { ns: ME.into(), id }).await)?;
        }
    }
    Ok(())
}

/// A member's nickname (in `me`, so only the user and their devices see
/// it): `name`, what the user calls it ("Neighbour's iPad"), and
/// `retifs:node`, the member's device, so the name is the same in every
/// library that device is in (a device has a writer key per library);
/// `retifs:writer`, the writer key (hex), names a member whose node isn't
/// known. Apps show it with the key's fingerprint words, never instead of
/// them, and decide nothing by it.
pub const NICKNAME_TYPE: &str = "retifs.nickname";

/// One nickname record.
#[derive(Debug, Clone, PartialEq)]
pub struct Nickname {
    pub id: String,
    pub name: String,
    /// Lowercase hex, if known.
    pub node: Option<String>,
    pub writer: Option<String>,
}

/// The nicknames.
pub async fn nicknames(c: &mut Client) -> Vec<Nickname> {
    let req = Request::Query { ns: ME.into(), terms: vec![format!("type={NICKNAME_TYPE}")], at: None };
    let Ok(Response::Records { records }) = c.call(req).await else { return Vec::new() };
    records
        .into_iter()
        .filter_map(|r| {
            let text = |k: &str| match r.attrs.iter().find(|(key, _)| key == k).map(|(_, v)| v) {
                Some(AttrValue::Str(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            Some(Nickname { name: text("name")?, node: text("retifs:node").map(|n| n.to_lowercase()), writer: text("retifs:writer").map(|w| w.to_lowercase()), id: r.id.clone() })
        })
        .collect()
}

/// A member's nickname: by its node (any library), else by its writer key.
pub fn nickname_of<'a>(nicks: &'a [Nickname], writer: &str, node: Option<&str>) -> Option<&'a str> {
    let (writer, node) = (writer.to_lowercase(), node.map(str::to_lowercase));
    node.as_ref()
        .and_then(|n| nicks.iter().find(|k| k.node.as_ref() == Some(n)))
        .or_else(|| nicks.iter().find(|k| k.writer.as_ref() == Some(&writer)))
        .map(|k| k.name.as_str())
}

/// Calls member `writer` (its key, hex) on `node` (its device, if known)
/// `name`; an empty name removes the nickname, and the member goes back to
/// its fingerprint words.
pub async fn set_nickname(c: &mut Client, writer: &str, node: Option<&str>, name: &str) -> Result<(), String> {
    let writer = writer.trim().to_lowercase();
    let node = node.map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty());
    let name = name.trim();
    // The member's earlier names go: by its node, or by this key.
    for k in nicknames(c).await {
        if (node.is_some() && k.node == node) || k.writer.as_ref() == Some(&writer) {
            err(c.call(Request::RecordDelete { ns: ME.into(), id: k.id }).await)?;
        }
    }
    if name.is_empty() {
        return Ok(());
    }
    let mut attrs = vec![
        ("type".to_string(), AttrValue::Str(NICKNAME_TYPE.into())),
        ("retifs:writer".to_string(), AttrValue::Str(writer)),
        ("name".to_string(), AttrValue::Str(name.into())),
    ];
    if let Some(n) = node {
        attrs.push(("retifs:node".to_string(), AttrValue::Str(n)));
    }
    err(c.call(Request::RecordCreate { ns: ME.into(), attrs, data: None }).await).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{nickname_of, Nickname};

    #[test]
    fn a_nickname_follows_the_device_into_every_library() {
        let k = |node: Option<&str>, writer: Option<&str>, name: &str| Nickname { id: name.into(), name: name.into(), node: node.map(String::from), writer: writer.map(String::from) };
        let nicks = [k(Some("aa"), Some("w1"), "iPad"), k(None, Some("w9"), "Old one")];
        assert_eq!(nickname_of(&nicks, "w2", Some("AA")), Some("iPad"), "another library: another key, the same node");
        assert_eq!(nickname_of(&nicks, "W9", None), Some("Old one"), "by key when the node isn't known");
        assert_eq!(nickname_of(&nicks, "w3", Some("bb")), None);
    }

    #[test]
    fn terms_split_on_commas() {
        assert_eq!(super::split_terms(" type=photo,album=Prangli 2026 ,, "), ["type=photo", "album=Prangli 2026"]);
    }
}

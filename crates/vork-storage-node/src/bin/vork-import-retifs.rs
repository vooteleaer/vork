//! vork-import-retifs: copies libraries kept by a retifs daemon into vØrk
//! storage, renaming as it goes (retifs.* types → vork.*, retifs: links and
//! attributes → vork:), the content read from retifs and written again (so
//! it's encrypted with vØrk storage's keys).
//!
//! It only reads from retifs (queries and reads; run it against a copy of
//! the store, as the import on TrueNAS does), and writes only to vØrk
//! storage. What it did is kept in a progress file, so a stopped import goes
//! on where it was.
//!
//!     vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE --list
//!     vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE --sample LIBRARY [N]
//!     vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE --import [LIBRARY]... [--progress FILE]

use std::collections::BTreeMap;
use std::path::PathBuf;

use libvork::{AttrValue, Client, RecordView, Request, Response};

fn fail(m: impl std::fmt::Display) -> ! {
    eprintln!("vork-import-retifs: {m}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!("vork-import-retifs --from RETIFS_SOCKET --to VORK_STORE (--list | --sample LIBRARY [N] | --import [LIBRARY]... | --verify) [--progress FILE]");
    std::process::exit(2)
}

async fn ask(c: &mut Client, r: Request) -> Response {
    match c.call(r).await {
        Ok(Response::Error { message }) => fail(message),
        Ok(r) => r,
        Err(e) => fail(e),
    }
}

/// The libraries the retifs daemon has.
async fn libraries(c: &mut Client) -> Vec<String> {
    match ask(c, Request::Namespaces).await {
        Response::Names { names } => names,
        other => fail(format!("unexpected answer to Namespaces: {other:?}")),
    }
}

async fn records(c: &mut Client, ns: &str) -> Vec<RecordView> {
    match ask(c, Request::Query { ns: ns.into(), terms: vec![], at: None }).await {
        Response::Records { records } => records,
        other => fail(format!("unexpected answer to Query: {other:?}")),
    }
}

fn attr<'a>(r: &'a RecordView, key: &str) -> Option<&'a AttrValue> {
    r.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn str_attr<'a>(r: &'a RecordView, key: &str) -> Option<&'a str> {
    match attr(r, key) {
        Some(AttrValue::Str(s)) => Some(s),
        _ => None,
    }
}

fn int_attr(r: &RecordView, key: &str) -> Option<i64> {
    match attr(r, key) {
        Some(AttrValue::Int(n)) => Some(*n),
        _ => None,
    }
}

async fn list(src: &mut Client) {
    for ns in libraries(src).await {
        let recs = records(src, &ns).await;
        let mut types: BTreeMap<String, usize> = BTreeMap::new();
        let (mut with_content, mut bytes) = (0usize, 0i64);
        for r in &recs {
            *types.entry(str_attr(r, "type").unwrap_or("(none)").to_string()).or_default() += 1;
            if !r.blocks.is_empty() {
                with_content += 1;
                bytes += int_attr(r, "size").unwrap_or(0);
            }
        }
        println!("{ns}: {} record(s), {with_content} with content, {:.1} MB", recs.len(), bytes as f64 / 1e6);
        let mut t: Vec<_> = types.into_iter().collect();
        t.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, n) in t.iter().take(12) {
            println!("  {n:>7} {k}");
        }
    }
}

async fn sample(src: &mut Client, ns: &str, n: usize) {
    let recs = records(src, ns).await;
    // A few of each type.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for r in recs {
        let t = str_attr(&r, "type").unwrap_or("(none)").to_string();
        let k = seen.entry(t).or_default();
        if *k >= n {
            continue;
        }
        *k += 1;
        println!("{} blocks={:?}", r.id, r.blocks);
        for (k, v) in &r.attrs {
            let v = v.to_string();
            println!("  {k} = {}", if v.len() > 120 { format!("{}…", &v[..v.char_indices().nth(120).map_or(v.len(), |(i, _)| i)]) } else { v });
        }
    }
}

// ---- importing ----------------------------------------------------------------

/// Libraries not taken: retifs' personal one (favourites of retifs' writers).
const SKIP_LIBRARIES: [&str; 1] = ["me"];
/// Records not taken: who held what on retifs' network.
const SKIP_TYPES: [&str; 1] = ["retifs.holders"];
/// Attributes not taken: derived blocks (thumbnails, previews: not copied)
/// and the extractor's mark (so vØrk storage's extractor makes them again);
/// size is the new content's own.
fn skip_key(k: &str) -> bool {
    k.starts_with("block:") || k == "extracted" || k == "size"
}

/// A retifs name as vØrk storage's.
fn rename_str(s: &str) -> String {
    for (old, new) in [("application/x-vnd.retifs.", "application/x-vnd.vork."), ("retifs-ch:", "vork-ch:"), ("retifs:", "vork:"), ("retifs.", "vork.")] {
        if let Some(rest) = s.strip_prefix(old) {
            return format!("{new}{rest}");
        }
    }
    s.to_string()
}

fn rename_value(v: &AttrValue) -> AttrValue {
    match v {
        AttrValue::Str(s) => AttrValue::Str(rename_str(s)),
        AttrValue::List(l) => AttrValue::List(l.iter().map(rename_value).collect()),
        other => other.clone(),
    }
}

fn translate(attrs: &[(String, AttrValue)]) -> Vec<(String, AttrValue)> {
    attrs
        .iter()
        .filter(|(k, _)| !skip_key(k))
        .map(|(k, v)| {
            let k = match k.strip_prefix("retifs:") {
                Some(rest) => format!("vork:{rest}"),
                None => k.clone(),
            };
            (k, rename_value(v))
        })
        .collect()
}

/// The whole content of `cap`, from the retifs daemon's own store only (no
/// nodes asked: it has no network, and asking would wait).
async fn read_all(src: &mut Client, _ns: &str, cap: &str, size: u64) -> Result<Vec<u8>, String> {
    let req = Request::Get { cap: cap.into(), from: vec![], links_per_node: 1, batch: 64 };
    match src.call(req).await.map_err(|e| e.to_string())? {
        Response::Get { data, .. } if data.len() as u64 == size => Ok(data),
        Response::Get { data, .. } => Err(format!("{} of {size} bytes here", data.len())),
        Response::Error { message } => Err(message),
        other => Err(format!("unexpected answer {other:?}")),
    }
}

/// The content's size: the file reference's last part (`…:size`), else the record's.
fn cap_size(r: &RecordView) -> Option<u64> {
    r.blocks.first().and_then(|c| c.rsplit(':').next()).and_then(|s| s.parse().ok()).or(int_attr(r, "size").map(|n| n as u64))
}

struct Progress {
    file: PathBuf,
    done: std::collections::HashSet<(String, String)>,
}

impl Progress {
    fn open(file: PathBuf) -> Progress {
        let done = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\t');
                Some((p.next()?.to_string(), p.next()?.to_string()))
            })
            .collect();
        Progress { file, done }
    }
    fn has(&self, ns: &str, id: &str) -> bool {
        self.done.contains(&(ns.to_string(), id.to_string()))
    }
    /// Kept at once: an import stopped now goes on from here.
    fn add(&mut self, ns: &str, src_id: &str, dst_id: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.file).unwrap_or_else(|e| fail(format!("{}: {e}", self.file.display())));
        writeln!(f, "{ns}\t{src_id}\t{dst_id}").unwrap_or_else(|e| fail(e));
        self.done.insert((ns.to_string(), src_id.to_string()));
    }
}

/// Records of up to this size go in transactions of many; larger ones each
/// on its own, the large ones streamed.
const SMALL: u64 = 256 << 10;
const BATCH_BYTES: usize = 32 << 20;
const BATCH: usize = 500;
const STREAM_OVER: u64 = 64 << 20;

async fn flush(dst: &mut Client, ns: &str, batch: &mut Vec<(String, Vec<(String, AttrValue)>, Option<Vec<u8>>)>, progress: &mut Progress) {
    if batch.is_empty() {
        return;
    }
    let steps = batch.iter().map(|(_, attrs, data)| libvork::TxStep::Create { attrs: attrs.clone(), data: data.clone() }).collect();
    let made = match ask(dst, Request::Transaction { ns: ns.into(), steps }).await {
        Response::Records { records } => records,
        other => fail(format!("unexpected answer to Transaction: {other:?}")),
    };
    if made.len() != batch.len() {
        fail(format!("{ns}: a transaction of {} made {} record(s)", batch.len(), made.len()));
    }
    for ((src_id, _, _), r) in batch.iter().zip(&made) {
        progress.add(ns, src_id, &r.id);
    }
    batch.clear();
}

async fn write_large(dst: &mut Client, ns: &str, attrs: Vec<(String, AttrValue)>, data: &[u8]) -> String {
    let id = match ask(dst, Request::RecordCreate { ns: ns.into(), attrs: attrs.clone(), data: None }).await {
        Response::Records { records } if !records.is_empty() => records[0].id.clone(),
        other => fail(format!("unexpected answer to RecordCreate: {other:?}")),
    };
    let session = match ask(dst, Request::WriteBegin { ns: ns.into(), id: id.clone(), truncate: true }).await {
        Response::Session { session, .. } => session,
        other => fail(format!("unexpected answer to WriteBegin: {other:?}")),
    };
    for (i, piece) in data.chunks(1 << 20).enumerate() {
        ask(dst, Request::WriteAt { session, offset: (i << 20) as u64, data: piece.to_vec() }).await;
    }
    ask(dst, Request::WriteCommit { session, attrs: vec![] }).await;
    id
}

async fn import(src: &mut Client, dst: &mut Client, only: &[String], progress: &mut Progress) {
    let mut names = libraries(src).await;
    names.retain(|n| !SKIP_LIBRARIES.contains(&n.as_str()) && (only.is_empty() || only.contains(n)));
    for ns in names {
        let recs = records(src, &ns).await;
        // The library, made here if it isn't yet (its address makes it).
        ask(dst, Request::NsAddress { ns: ns.clone() }).await;
        let (mut copied, mut skipped, mut missing, mut before) = (0usize, 0usize, Vec::new(), 0usize);
        let mut batch = Vec::new();
        let mut batch_bytes = 0usize;
        let total = recs.len();
        for (i, r) in recs.iter().enumerate() {
            if progress.has(&ns, &r.id) {
                before += 1;
                continue;
            }
            if str_attr(r, "type").is_some_and(|t| SKIP_TYPES.contains(&t)) {
                skipped += 1;
                continue;
            }
            let attrs = translate(&r.attrs);
            let size = cap_size(r).unwrap_or(0);
            let data = match r.blocks.first() {
                None => None,
                Some(cap) => match read_all(src, &ns, cap, size).await {
                    Ok(d) => Some(d),
                    Err(e) => {
                        missing.push((r.id.clone(), str_attr(r, "path").or(str_attr(r, "name")).unwrap_or("").to_string(), e));
                        continue;
                    }
                },
            };
            let len = data.as_ref().map_or(0, |d| d.len());
            if (len as u64) <= SMALL {
                batch.push((r.id.clone(), attrs, data));
                batch_bytes += len;
                if batch.len() >= BATCH || batch_bytes >= BATCH_BYTES {
                    flush(dst, &ns, &mut batch, progress).await;
                    batch_bytes = 0;
                }
            } else {
                flush(dst, &ns, &mut batch, progress).await;
                batch_bytes = 0;
                let data = data.unwrap();
                let id = if (len as u64) > STREAM_OVER {
                    write_large(dst, &ns, attrs, &data).await
                } else {
                    match ask(dst, Request::RecordCreate { ns: ns.clone(), attrs, data: Some(data) }).await {
                        Response::Records { records } if !records.is_empty() => records[0].id.clone(),
                        other => fail(format!("unexpected answer to RecordCreate: {other:?}")),
                    }
                };
                progress.add(&ns, &r.id, &id);
            }
            copied += 1;
            if (i + 1) % 1000 == 0 {
                println!("{ns}: {} of {total} looked at, {copied} copied, {} not here", i + 1, missing.len());
            }
        }
        flush(dst, &ns, &mut batch, progress).await;
        println!("{ns}: {total} record(s): {copied} copied now, {before} before, {skipped} left out (retifs' own), {} whose content isn't here", missing.len());
        for (id, name, e) in missing.iter().take(5) {
            println!("  not here: {id} {name} ({e})");
        }
    }
}

/// After an import: each library's records here against retifs', and every
/// copied file's size against its source's.
async fn verify(src: &mut Client, dst: &mut Client, progress: &Progress) {
    let file = std::fs::read_to_string(&progress.file).unwrap_or_default();
    let mut map: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for l in file.lines() {
        let p: Vec<&str> = l.split('\t').collect();
        if p.len() == 3 {
            map.entry(p[0].to_string()).or_default().insert(p[1].to_string(), p[2].to_string());
        }
    }
    for (ns, pairs) in map {
        let from: BTreeMap<String, RecordView> = records(src, &ns).await.into_iter().map(|r| (r.id.clone(), r)).collect();
        let here: BTreeMap<String, RecordView> = records(dst, &ns).await.into_iter().map(|r| (r.id.clone(), r)).collect();
        let (mut ok, mut bad) = (0usize, Vec::new());
        for (s, d) in &pairs {
            match (from.get(s), here.get(d)) {
                (Some(a), Some(b)) => {
                    let want = if a.blocks.is_empty() { 0 } else { cap_size(a).unwrap_or(0) };
                    let got = if b.blocks.is_empty() { 0 } else { cap_size(b).unwrap_or(0) };
                    let attrs_ok = translate(&a.attrs).iter().all(|(k, v)| b.attrs.iter().any(|(k2, v2)| k2 == k && v2 == v));
                    if want == got && attrs_ok { ok += 1 } else { bad.push(format!("{s} → {d}: size {want}/{got}, attributes {}", if attrs_ok { "same" } else { "differ" })) }
                }
                (_, None) => bad.push(format!("{s} → {d}: not here")),
                (None, _) => {}
            }
        }
        println!("{ns}: {ok} of {} copied record(s) checked: same size and attributes; {} here in all", pairs.len(), here.len());
        for b in bad.iter().take(5) {
            println!("  differs: {b}");
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let val = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let from = val("--from").unwrap_or_else(|| usage());
    let to: PathBuf = val("--to").unwrap_or_else(|| usage()).into();
    let mut src = Client::connect_endpoint(&from).await.unwrap_or_else(|e| fail(e));
    if args.iter().any(|a| a == "--list") {
        return list(&mut src).await;
    }
    if let Some(ns) = val("--sample") {
        let n = args.iter().position(|a| a == "--sample").and_then(|i| args.get(i + 2)).and_then(|s| s.parse().ok()).unwrap_or(2);
        return sample(&mut src, &ns, n).await;
    }
    let mut dst = Client::connect(&to).await.unwrap_or_else(|e| fail(e));
    let mut progress = Progress::open(val("--progress").map(PathBuf::from).unwrap_or_else(|| to.join("import-retifs.tsv")));
    if args.iter().any(|a| a == "--verify") {
        return verify(&mut src, &mut dst, &progress).await;
    }
    if let Some(i) = args.iter().position(|a| a == "--import") {
        let only: Vec<String> = args[i + 1..].iter().take_while(|a| !a.starts_with("--")).cloned().collect();
        import(&mut src, &mut dst, &only, &mut progress).await;
        return verify(&mut src, &mut dst, &progress).await;
    }
    usage()
}

//! An application adding pages to the node's dashboard: two pages, one that
//! asks the application for something (`vork.call`) and shows the answer.
//!
//! `cargo run -p vork-api --example pages -- [endpoint]` (default: the
//! machine's node), then open the dashboard: "Hello" is in its menu while
//! this runs, and stays there as stopped after it ends.

use vork_api::{Client, Event, default_endpoint};

const OVERVIEW: &str = r##"
<p>This page comes from an application on this machine, shown by the node's dashboard.</p>
<p><button id="b" style="font:inherit;padding:6px 12px;border-radius:8px;border:1px solid var(--line2);background:var(--panel2);color:var(--text)">Ask the application the time</button></p>
<p id="out" style="color:var(--dim)">Not asked yet.</p>
<p><a href="#" id="more">The other page</a></p>
<script>
document.getElementById("b").onclick = async () => {
  const r = await vork.call("/time", { asked: Date.now() });
  document.getElementById("out").textContent = "Status " + r.status + ": " + r.body;
};
document.getElementById("more").onclick = (e) => { e.preventDefault(); vork.go("/more"); };
// Out of the sandbox: this must fail.
try { parent.document.title; document.body.insertAdjacentHTML("beforeend", "<p>NOT SANDBOXED</p>"); } catch (e) { document.body.insertAdjacentHTML("beforeend", "<p style='color:var(--ok)'>Sandboxed: the dashboard can't be reached from here.</p>"); }
</script>"##;

const MORE: &str = r##"<p>A second page. <a href="#" onclick="vork.go('/');return false">Back</a></p>"##;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let endpoint = std::env::args().nth(1).unwrap_or_else(default_endpoint);
    let (client, mut events) = Client::connect(&endpoint).await?;
    client.pages("hello", "Hello", vec![("/".into(), "Overview".into()), ("/more".into(), "More".into())])?;
    println!("pages added at {endpoint}; Ctrl-C to stop");
    while let Some(ev) = events.recv().await {
        match ev {
            Event::Page { token, path, body, .. } => {
                println!("asked for {path} ({} bytes)", body.len());
                let (status, out) = match path.as_str() {
                    "/" => (0, OVERVIEW.to_string()),
                    "/more" => (0, MORE.to_string()),
                    "/time" => (0, format!("{{\"unix\":{},\"you_sent\":{}}}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(), String::from_utf8_lossy(&body))),
                    _ => (1, "no such page".to_string()),
                };
                client.respond(token, status, out.into_bytes())?;
            }
            Event::Gone => break,
            _ => {}
        }
    }
    Ok(())
}

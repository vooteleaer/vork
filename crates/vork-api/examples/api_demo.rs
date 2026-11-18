//! Tries the local API against a running node.
//!
//!     api_demo ENDPOINT serve NAME          # hosts NAME with a fresh identity, answers /echo
//!     api_demo ENDPOINT ask DEST BYTES [identify-as-NAME]
use std::time::{Duration, Instant};
use vork_api::{Client, Event, STATUS_OK, STATUS_NO_PATH};

fn secret() -> [u8; 64] {
    use std::io::Read;
    let mut s = [0u8; 64];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut s).unwrap();
    s
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (client, mut events) = Client::connect(&a[1]).await?;
    println!("node {}", hex(&client.node));
    match a[2].as_str() {
        "serve" => {
            let dest = client.register(secret(), &a[3]).await?;
            client.announce(dest, b"demo")?;
            println!("serving {} as {}", a[3], hex(&dest));
            while let Some(ev) = events.recv().await {
                match ev {
                    Event::Incoming { token, path, body, requester, nonce, .. } => {
                        println!("request {path} ({} bytes) from {} in session {}", body.len(), requester.map(|r| hex(&r)).unwrap_or("anonymous".into()), hex(&nonce[..4]));
                        let status = if path == "/echo" { STATUS_OK } else { STATUS_NO_PATH };
                        client.respond(token, status, body)?;
                    }
                    Event::Gone => break,
                    _ => {}
                }
            }
        }
        "ask" => {
            let dest: [u8; 16] = unhex(&a[3]);
            let n: usize = a[4].parse().unwrap();
            let known = client.find(dest, Duration::from_secs(10)).await?.expect("no path");
            println!("found: {} hops, app data {:?}", known.hops, String::from_utf8_lossy(&known.app_data));
            let (session, nonce) = client.open(dest).await?;
            println!("session {session}, nonce {}", hex(&nonce[..4]));
            if let Some(name) = a.get(5) {
                let mine = client.register(secret(), name).await?;
                client.identify(session, mine)?;
            }
            for _ in 0..3 {
                let t = Instant::now();
                let (status, body) = client.request(session, "/echo", vec![7u8; n]).await?;
                println!("status {status}, {} bytes back in {:?}", body.len(), t.elapsed());
                assert_eq!(body, vec![7u8; n]);
            }
        }
        _ => panic!("serve or ask"),
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

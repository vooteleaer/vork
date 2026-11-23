//! vØrk storage on the machine's vØrk node (ADR-0022): identities, the
//! destinations storage answers for, and the connection to the node's local
//! API, which comes back by itself when the node restarts.
//!
//! The destination names (`retifs.node`, `retifs.library`, ...) are part of
//! the storage format: stored node and library addresses are made from them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use std::{future::Future, io, path::Path};

use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use vork_api::{Client, Event, Known};

pub const APP_NAME: &str = "vork";
/// A node serving storage requests.
pub const NODE_NAME: &str = "vork.node";
/// A node that only asks.
pub const CLIENT_NAME: &str = "vork.client";
/// The node's public libraries (ADR-0029).
pub const LIBRARY_NAME: &str = "vork.library";
/// A node keeping the user's keyring (ADR-0033).
pub const KEYRING_NAME: &str = "vork.keyring";

/// An identity's secret: X25519 secret ‖ Ed25519 seed (the storage and vØrk key file).
pub type IdentitySecret = [u8; 64];

/// A destination's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DestinationHash(pub [u8; 16]);

impl DestinationHash {
    pub fn new(b: [u8; 16]) -> Self {
        DestinationHash(b)
    }
    pub fn from_slice(b: &[u8]) -> Option<Self> {
        Some(DestinationHash(b.try_into().ok()?))
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Runs the future made by `make` on a multi-threaded runtime with large
/// stacks (storage's futures are deep; Windows' main thread has 1 MB).
pub fn block_on<T, Fut>(make: impl FnOnce() -> Fut + Send + 'static) -> T
where
    T: Send + 'static,
    Fut: Future<Output = T>,
{
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 << 20)
                .build()
                .expect("tokio runtime")
                .block_on(make())
        })
        .expect("spawn runtime thread")
        .join()
        .expect("runtime thread panicked")
}

/// Loads the node identity, creating it on first use, so the node keeps one address.
pub fn load_or_create_identity(path: &Path) -> io::Result<IdentitySecret> {
    if let Ok(raw) = std::fs::read(path) {
        return raw.try_into().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "identity file has the wrong length"));
    }
    let secret = new_identity();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, secret)?;
    Ok(secret)
}

pub fn new_identity() -> IdentitySecret {
    let mut s = [0u8; 64];
    getrandom::fill(&mut s).expect("OS entropy for identity generation");
    s
}

/// The address of destination `name` of the identity with public key `public` (X25519 ‖ Ed25519).
pub fn destination_of(public: &[u8; 64], name: &str) -> DestinationHash {
    let id: [u8; 16] = Sha256::digest(public)[..16].try_into().unwrap();
    let mut h = Sha256::new();
    h.update(&Sha256::digest(name.as_bytes())[..10]);
    h.update(id);
    DestinationHash(h.finalize()[..16].try_into().unwrap())
}

/// The address of destination `name` of the identity with this secret.
pub fn destination_hash(identity: &IdentitySecret, name: &str) -> DestinationHash {
    destination_of(&vork_core::lease::identity_public(identity), name)
}

pub fn parse_destination(hex_str: &str) -> Option<DestinationHash> {
    let bytes = hex::decode(hex_str.trim().trim_start_matches('<').trim_end_matches('>')).ok()?;
    DestinationHash::from_slice(&bytes)
}

pub fn destination_hex(hash: &DestinationHash) -> String {
    hex::encode(hash.as_bytes())
}

// ---- the connection to the node ----------------------------------------------------

/// What arrives from the node.
pub enum NetEvent {
    Heard { dest: DestinationHash, known: Known },
    Incoming { token: u64, dest: DestinationHash, session: u32, nonce: [u8; 32], requester: Option<[u8; 16]>, path: String, body: Vec<u8> },
    /// The node came back after a restart: destinations are registered and
    /// announced again, but sessions over the old connection are gone.
    Reconnected,
}

struct Inner {
    endpoint: String,
    client: RwLock<Client>,
    registered: Mutex<Vec<(IdentitySecret, String)>>,
    app_data: Mutex<HashMap<[u8; 16], Vec<u8>>>,
    subscribed: AtomicBool,
    devices: Mutex<Vec<[u8; 16]>>,
}

/// The machine's vØrk node, through its local API. Cheap to clone.
#[derive(Clone)]
pub struct Net(Arc<Inner>);

impl Net {
    /// Connects to the node at `endpoint`, waiting for one to be running.
    pub async fn connect(endpoint: &str, mut waiting: impl FnMut(&str)) -> (Net, mpsc::UnboundedReceiver<NetEvent>) {
        let (client, events) = loop {
            match Client::connect(endpoint).await {
                Ok(c) => break c,
                Err(e) => {
                    waiting(&format!("no vØrk node at {endpoint} ({e})"));
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        };
        let net = Net(Arc::new(Inner {
            endpoint: endpoint.to_string(),
            client: RwLock::new(client),
            registered: Mutex::default(),
            app_data: Mutex::default(),
            subscribed: AtomicBool::new(false),
            devices: Mutex::new(Vec::new()),
        }));
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(net.clone().pump(events, tx));
        (net, rx)
    }

    async fn pump(self, mut events: mpsc::UnboundedReceiver<Event>, tx: mpsc::UnboundedSender<NetEvent>) {
        loop {
            while let Some(ev) = events.recv().await {
                let out = match ev {
                    Event::Heard { dest, known } => NetEvent::Heard { dest: DestinationHash(dest), known },
                    Event::Incoming { token, dest, session, nonce, requester, path, body } => {
                        NetEvent::Incoming { token, dest: DestinationHash(dest), session, nonce, requester, path, body }
                    }
                    Event::Closed { .. } => continue,
                    // Dashboard pages: not served yet.
                    Event::Page { .. } => continue,
                    Event::Gone => break,
                };
                if tx.send(out).is_err() {
                    return;
                }
            }
            // The node went away: wait for it, then register everything again.
            eprintln!("the vØrk node went away; waiting for it");
            let (client, ev) = loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if let Ok(c) = Client::connect(&self.0.endpoint).await {
                    break c;
                }
            };
            *self.0.client.write().unwrap() = client.clone();
            let registered = self.0.registered.lock().unwrap().clone();
            for (secret, name) in registered {
                let _ = client.register(secret, &name).await;
            }
            if self.0.subscribed.load(Ordering::Relaxed) {
                let _ = client.subscribe();
            }
            let devices = self.0.devices.lock().unwrap().clone();
            if !devices.is_empty() {
                let _ = client.devices(devices);
            }
            let data: Vec<([u8; 16], Vec<u8>)> = self.0.app_data.lock().unwrap().iter().map(|(k, v)| (*k, v.clone())).collect();
            for (dest, d) in data {
                let _ = client.announce(dest, &d);
            }
            events = ev;
            if tx.send(NetEvent::Reconnected).is_err() {
                return;
            }
        }
    }

    pub fn client(&self) -> Client {
        self.0.client.read().unwrap().clone()
    }

    /// The node's own address.
    pub fn node_address(&self) -> [u8; 16] {
        self.client().node
    }

    /// Hosts destination `name` of `identity` on the node (again after a restart too).
    pub async fn register(&self, identity: IdentitySecret, name: &str) -> io::Result<DestinationHash> {
        let dest = self.client().register(identity, name).await?;
        self.0.registered.lock().unwrap().push((identity, name.to_string()));
        Ok(DestinationHash(dest))
    }

    /// Announces `dest` now, and from now on, with this app data.
    pub fn announce(&self, dest: DestinationHash, app_data: &[u8]) -> io::Result<()> {
        self.0.app_data.lock().unwrap().insert(dest.0, app_data.to_vec());
        self.client().announce(dest.0, app_data)
    }

    /// Announces `dest` again with the app data it has.
    pub fn announce_again(&self, dest: DestinationHash) -> io::Result<()> {
        let data = self.0.app_data.lock().unwrap().get(&dest.0).cloned().unwrap_or_default();
        self.client().announce(dest.0, &data)
    }

    /// The owner's other devices' destinations: the node gives sessions with
    /// them direct links (ADR-0011). Told again after a reconnect.
    pub fn set_devices(&self, devices: Vec<DestinationHash>) -> io::Result<()> {
        let list: Vec<[u8; 16]> = devices.into_iter().map(|d| d.0).collect();
        let mut held = self.0.devices.lock().unwrap();
        if *held == list {
            return Ok(());
        }
        *held = list.clone();
        drop(held);
        self.client().devices(list)
    }

    /// Every announce the node hears arrives as `NetEvent::Heard`.
    pub fn subscribe(&self) -> io::Result<()> {
        self.0.subscribed.store(true, Ordering::Relaxed);
        self.client().subscribe()
    }

    /// The public key (X25519 ‖ Ed25519) of the identity behind `dest`, if an announce told.
    pub async fn destination_identity(&self, dest: DestinationHash) -> Option<[u8; 64]> {
        self.client().recall(dest.0).await.ok().flatten().map(|k| k.public)
    }

    /// Answers a request that arrived as `NetEvent::Incoming`.
    pub fn respond(&self, token: u64, status: u8, body: Vec<u8>) {
        let _ = self.client().respond(token, status, body);
    }
}

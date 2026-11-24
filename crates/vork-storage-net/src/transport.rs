//! The transport seam (ADR-0003): vØrk storage talks to the network only
//! through this trait, so fetch logic can be tested with an in-memory network.
//! Its implementation is the machine's vØrk node (ADR-0022).

use std::{future::Future, time::Duration, time::Instant};

use crate::node::{DestinationHash, Net};

pub trait Transport: Clone + Send + 'static {
    /// A remote node's address.
    type Peer: Copy + Send + Sync + 'static;
    /// An established, encrypted session with a peer.
    type Link: Copy + Send + Sync + 'static;

    /// Finds a path to `peer` and opens a link.
    fn connect(&self, peer: Self::Peer) -> impl Future<Output = Result<Self::Link, String>> + Send;

    /// A value both ends of `link` know and no other link shares (the vØrk
    /// session nonce). Proofs of access are bound to it so they can't be replayed elsewhere.
    fn link_nonce(&self, link: Self::Link) -> Vec<u8>;

    /// Sends a request to `endpoint` on `link` and waits for the response.
    fn request(
        &self,
        link: Self::Link,
        endpoint: &'static str,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
}

/// A session with another node: the node's session number and its nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinkId {
    pub session: u32,
    pub nonce: [u8; 32],
}

impl LinkId {
    pub fn as_bytes(&self) -> &[u8] {
        &self.nonce
    }
}

/// Storage over the machine's vØrk node.
#[derive(Clone)]
pub struct VorkTransport {
    pub net: Net,
    /// Our destination that identifies us on links (`vork.node`).
    pub identify_as: DestinationHash,
    pub path_timeout: Duration,
}

impl VorkTransport {
    pub fn new(net: Net, identify_as: DestinationHash) -> Self {
        VorkTransport { net, identify_as, path_timeout: Duration::from_secs(60) }
    }

    /// Makes sure a path to `peer` is known: a route already held is enough;
    /// only otherwise is the network asked.
    pub async fn ensure_path(&self, peer: DestinationHash) -> Result<(), String> {
        if self.net.client().recall(peer.0).await.ok().flatten().is_some() {
            return Ok(());
        }
        self.request_path(peer).await
    }

    /// How many hops the route to `peer` now has, if one is known.
    pub async fn hops(&self, peer: DestinationHash) -> Option<u8> {
        self.net.client().recall(peer.0).await.ok().flatten().map(|k| k.hops)
    }

    /// Closes `link` (a request still on it fails).
    pub fn close(&self, link: LinkId) {
        let _ = self.net.client().close(link.session);
    }

    /// Asks the network for a path.
    pub async fn request_path(&self, peer: DestinationHash) -> Result<(), String> {
        match self.net.client().find(peer.0, self.path_timeout).await {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(format!("no path within {:?}", self.path_timeout)),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Asks for a fresh path after a link over the known one failed.
    pub async fn refresh_path(&self, peer: DestinationHash) -> Result<(), String> {
        self.request_path(peer).await
    }

    /// Like [`Transport::request`], with the response awaited up to `timeout`.
    pub async fn request_waiting(&self, link: LinkId, endpoint: &'static str, body: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String> {
        tokio::time::timeout(timeout, self.request(link, endpoint, body)).await.map_err(|_| format!("{endpoint}: timed out"))?
    }

    /// A request to any path, with the response awaited up to `timeout`.
    pub async fn request_any_path(&self, link: LinkId, path: &str, body: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String> {
        let c = self.net.client();
        match tokio::time::timeout(timeout, c.request(link.session, path, body)).await {
            Ok(Ok((vork_api::STATUS_OK, b))) => Ok(b),
            Ok(Ok((status, _))) => Err(format!("{path}: declined ({status})")),
            Ok(Err(e)) => Err(format!("{path}: {e}")),
            Err(_) => Err(format!("{path}: timed out")),
        }
    }

    /// Reveals this node's identity to the other end of `link`, so it can
    /// tell who is asking. (The identity hash is our `identify_as` destination's.)
    pub async fn identify(&self, link: LinkId, _identity_hash: [u8; 16]) -> Result<(), String> {
        self.net.client().identify(link.session, self.identify_as.0).map_err(|e| format!("identify: {e}"))
    }

    /// Opens a link to a peer whose path is already known.
    pub async fn link(&self, peer: DestinationHash) -> Result<LinkId, String> {
        self.link_with_rtt(peer).await.map(|(link, _)| link)
    }

    /// Like [`link`](Self::link), also returning the round trip in milliseconds
    /// of a first exchange over it (session setup included).
    pub async fn link_with_rtt(&self, peer: DestinationHash) -> Result<(LinkId, u64), String> {
        let c = self.net.client();
        let t = Instant::now();
        let (session, nonce) = c.open(peer.0).await.map_err(|e| format!("link failed: {e}"))?;
        // Any path will do: the answer (even "no such path") measures the trip.
        let probe = tokio::time::timeout(Duration::from_secs(30), c.request(session, "/vork/rtt", Vec::new())).await;
        match probe {
            Ok(Ok(_)) => Ok((LinkId { session, nonce }, t.elapsed().as_millis() as u64)),
            Ok(Err(e)) => Err(format!("link failed: {e}")),
            Err(_) => Err("link failed: no answer within 30 s".into()),
        }
    }
}

impl Transport for VorkTransport {
    type Peer = DestinationHash;
    type Link = LinkId;

    fn link_nonce(&self, link: LinkId) -> Vec<u8> {
        link.nonce.to_vec()
    }

    fn connect(&self, peer: DestinationHash) -> impl Future<Output = Result<LinkId, String>> + Send {
        let this = self.clone();
        async move {
            this.ensure_path(peer).await?;
            this.link(peer).await
        }
    }

    fn request(&self, link: LinkId, endpoint: &'static str, body: Vec<u8>) -> impl Future<Output = Result<Vec<u8>, String>> + Send {
        let c = self.net.client();
        async move {
            match c.request(link.session, endpoint, body).await {
                Ok((vork_api::STATUS_OK, b)) => Ok(b),
                Ok((status, _)) => Err(format!("{endpoint}: declined ({status})")),
                Err(e) => Err(format!("{endpoint}: {e}")),
            }
        }
    }
}

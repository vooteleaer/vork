//! Serving requests that arrive for storage's destinations: each endpoint is
//! a type with a path and a handler, dispatched by [`dispatch`].

use std::cell::Cell;
use std::future::Future;

use crate::node::Net;
use crate::transport::LinkId;

/// Why a request got no answer.
#[derive(Debug)]
pub enum Decline {
    /// No answer at all (or one sent later through the respond token).
    Ignore,
}

pub enum RequestEndpointPolicy {
    AllowAll,
}

/// What a handler may ask of the node while handling (nothing yet).
pub trait NodeApi {}
impl NodeApi for Net {}

/// The address of the identity that identified on the link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityHash(pub [u8; 16]);

impl IdentityHash {
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Answers a request later, from another task.
#[derive(Clone, Copy, Debug)]
pub struct RespondToken {
    pub link_id: LinkId,
    pub token: u64,
}

pub struct RequestContext<'a, S> {
    pub data: &'a [u8],
    pub state: &'a S,
    pub requester: Option<IdentityHash>,
    token: RespondToken,
    deferred: &'a Cell<bool>,
    answer: &'a Cell<Option<Vec<u8>>>,
}

impl<S> RequestContext<'_, S> {
    /// The token to answer later with (`Net::respond`); also carries the link.
    pub fn respond_token(&self) -> RespondToken {
        self.deferred.set(true);
        self.token
    }

    pub fn respond(&mut self, body: Vec<u8>) -> Result<(), Decline> {
        self.answer.set(Some(body));
        Ok(())
    }
}

pub trait RequestEndpoint<S> {
    const ENDPOINT_ID: &'static str;
    const POLICY: RequestEndpointPolicy;
    fn handle(context: RequestContext<'_, S>, node: &impl NodeApi) -> impl Future<Output = Result<(), Decline>>;
}

/// One request that arrived (`NetEvent::Incoming`).
pub struct Incoming {
    pub token: u64,
    pub session: u32,
    pub nonce: [u8; 32],
    pub requester: Option<[u8; 16]>,
    pub path: String,
    pub body: Vec<u8>,
}

/// Runs endpoint `E` on `req` if the path is its own: answers now, or leaves
/// the answer to whoever took the respond token. Returns whether it was `E`'s.
pub async fn dispatch<S, E: RequestEndpoint<S>>(net: &Net, state: &S, req: &Incoming) -> bool {
    if req.path != E::ENDPOINT_ID {
        return false;
    }
    let deferred = Cell::new(false);
    let answer = Cell::new(None);
    let context = RequestContext {
        data: &req.body,
        state,
        requester: req.requester.map(IdentityHash),
        token: RespondToken { link_id: LinkId { session: req.session, nonce: req.nonce }, token: req.token },
        deferred: &deferred,
        answer: &answer,
    };
    let _ = E::handle(context, net).await;
    match answer.take() {
        Some(body) => net.respond(req.token, vork_api::STATUS_OK, body),
        // Declined without taking the token: say so now rather than leave the asker waiting.
        None if !deferred.get() => net.respond(req.token, vork_api::STATUS_REFUSED, Vec::new()),
        None => {}
    }
    true
}

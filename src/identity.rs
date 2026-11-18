//! Node identity: an Ed25519 signing key and an X25519 key (the WireGuard
//! static key), as in RNS. The address of a node is the truncated hash of both
//! public keys, so only the holder of the signing key can announce it.

use anyhow::{Context, Result, bail};
use boringtun::x25519::{PublicKey, StaticSecret};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const HASH_LEN: usize = 16;
pub type Hash = [u8; HASH_LEN];

/// Domain-separation labels: every signature covers `label ‖ message`.
pub const LABEL_BIND: &[u8] = b"vork/v0/bind";
pub const LABEL_ANNOUNCE: &[u8] = b"vork/v0/announce";

pub struct Identity {
    pub sign: SigningKey,
    pub dh: StaticSecret,
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let mut seed = [0u8; 64];
        getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("OS entropy: {e}"))?;
        let sign = SigningKey::from_bytes(seed[..32].try_into().unwrap());
        let dh = StaticSecret::from(<[u8; 32]>::try_from(&seed[32..]).unwrap());
        Ok(Identity { sign, dh })
    }

    /// The key file holds 64 hex-encoded bytes: Ed25519 seed ‖ X25519 secret.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let bytes = hex::decode(text.trim()).context("key file is not hex")?;
        if bytes.len() != 64 {
            bail!("key file must hold 64 bytes, has {}", bytes.len());
        }
        Ok(Identity {
            sign: SigningKey::from_bytes(bytes[..32].try_into().unwrap()),
            dh: StaticSecret::from(<[u8; 32]>::try_from(&bytes[32..]).unwrap()),
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut bytes = self.sign.to_bytes().to_vec();
        bytes.extend_from_slice(&self.dh.to_bytes());
        std::fs::write(path, hex::encode(bytes))?;
        Ok(())
    }

    pub fn ed_pub(&self) -> [u8; 32] {
        self.sign.verifying_key().to_bytes()
    }

    pub fn x_pub(&self) -> PublicKey {
        PublicKey::from(&self.dh)
    }

    pub fn hash(&self) -> Hash {
        node_hash(&self.ed_pub(), self.x_pub().as_bytes())
    }

    pub fn sign(&self, label: &[u8], msg: &[u8]) -> [u8; 64] {
        let mut m = label.to_vec();
        m.extend_from_slice(msg);
        self.sign.sign(&m).to_bytes()
    }
}

pub fn node_hash(ed_pub: &[u8; 32], x_pub: &[u8; 32]) -> Hash {
    let mut h = Sha256::new();
    h.update(ed_pub);
    h.update(x_pub);
    h.finalize()[..HASH_LEN].try_into().unwrap()
}

pub fn verify(ed_pub: &[u8; 32], label: &[u8], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(ed_pub) else { return false };
    let mut m = label.to_vec();
    m.extend_from_slice(msg);
    key.verify(&m, &Signature::from_bytes(sig)).is_ok()
}

#[allow(dead_code)] // for the overlay IPv6 compatibility view (phase 3)
/// Overlay IPv6 address of a hash: `fd` followed by the first 15 bytes.
/// The compatibility view for ordinary IP applications; never used for routing.
pub fn overlay_addr(hash: &Hash) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0] = 0xfd;
    a[1..].copy_from_slice(&hash[..15]);
    a
}

pub fn short(hash: &Hash) -> String {
    hex::encode(&hash[..4])
}

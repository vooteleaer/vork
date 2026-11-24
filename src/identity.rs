//! Identity: an X25519 key (the WireGuard static key) and an Ed25519 signing
//! key. The address is the truncated hash of both public keys, X25519 first,
//! so only the holder of the signing key can announce it. Hashes, key files
//! and named destinations are the ones vØrk storage's format already uses
//! (docs/storage/02-irreversible-decisions.md), so a storage identity is a
//! vØrk identity and its stored addresses stay valid.

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

    /// From the 64 secret bytes: X25519 secret ‖ Ed25519 seed.
    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        Identity {
            dh: StaticSecret::from(<[u8; 32]>::try_from(&bytes[..32]).unwrap()),
            sign: SigningKey::from_bytes(bytes[32..].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self) -> [u8; 64] {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&self.dh.to_bytes());
        bytes[32..].copy_from_slice(&self.sign.to_bytes());
        bytes
    }

    /// The key file holds the 64 secret bytes as they are (X25519 secret ‖
    /// Ed25519 seed), like a storage identity file. Prototype key files of
    /// 128 hex digits (Ed25519 seed ‖ X25519 secret) are still read.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if let Ok(bytes) = <[u8; 64]>::try_from(raw.as_slice()) {
            return Ok(Self::from_bytes(&bytes));
        }
        let text = String::from_utf8_lossy(&raw);
        let old = hex::decode(text.trim()).ok().filter(|b| b.len() == 64);
        let Some(old) = old else { bail!("{}: not a key file (64 bytes)", path.display()) };
        Ok(Identity {
            sign: SigningKey::from_bytes(old[..32].try_into().unwrap()),
            dh: StaticSecret::from(<[u8; 32]>::try_from(&old[32..]).unwrap()),
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_bytes())?;
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

/// An identity's address: SHA-256(X25519 public ‖ Ed25519 public)[0..16].
pub fn node_hash(ed_pub: &[u8; 32], x_pub: &[u8; 32]) -> Hash {
    let mut h = Sha256::new();
    h.update(x_pub);
    h.update(ed_pub);
    h.finalize()[..HASH_LEN].try_into().unwrap()
}

pub const NAME_HASH_LEN: usize = 10;

/// A service's name as it enters addresses: SHA-256(name)[0..10].
pub fn name_hash(name: &str) -> [u8; NAME_HASH_LEN] {
    Sha256::digest(name.as_bytes())[..NAME_HASH_LEN].try_into().unwrap()
}

/// The address of a named service of an identity (a destination):
/// SHA-256(name hash ‖ identity address)[0..16].
pub fn destination_hash(identity: &Hash, name: &str) -> Hash {
    destination_from_name_hash(identity, &name_hash(name))
}

pub fn destination_from_name_hash(identity: &Hash, name_hash: &[u8; NAME_HASH_LEN]) -> Hash {
    let mut h = Sha256::new();
    h.update(name_hash);
    h.update(identity);
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

#[cfg(test)]
mod tests {
    use super::*;

    // The YubiKey identity from docs/storage/recon/yubikey.md, checked with Python RNS.
    const X: &str = "4f30f06604f14fd9059eebce00f645063a72ca45bf94ab18d54aa10be8f9d852";
    const ED: &str = "bb3e8a8c6af96b39a9a94894604fd0a5cc6bbe7a2a404cad8a92e37190970f63";

    #[test]
    fn addresses_match_storage() {
        let x: [u8; 32] = hex::decode(X).unwrap().try_into().unwrap();
        let ed: [u8; 32] = hex::decode(ED).unwrap().try_into().unwrap();
        let id = node_hash(&ed, &x);
        assert_eq!(hex::encode(id), "12e9125e209f8c0f67acc1fe70b9e4f1");
        assert_eq!(hex::encode(destination_hash(&id, "vork.node")), "123f6a32d38d960defa70e5531a95317");
    }

    #[test]
    fn key_files_round_trip_and_old_ones_load() {
        let dir = std::env::temp_dir().join(format!("vork-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = Identity::generate().unwrap();
        let path = dir.join("node.key");
        id.save(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap().len(), 64);
        assert_eq!(Identity::load(&path).unwrap().hash(), id.hash());
        let mut old = id.sign.to_bytes().to_vec();
        old.extend_from_slice(&id.dh.to_bytes());
        std::fs::write(&path, hex::encode(old)).unwrap();
        assert_eq!(Identity::load(&path).unwrap().hash(), id.hash());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

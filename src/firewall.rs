//! The walled garden (ADR-0027): what reaches this node's own services over
//! vØrk. The owner's devices reach everything; other nodes only what `allow`
//! opens, and the answers in conversations this node began. Relaying for
//! others isn't touched: that never reaches the interface.
//!
//! Rules (`allow = [...]` in vork.toml):
//! - `tcp/80`, `udp/5000-5010`: those ports, for every vØrk node;
//! - `tcp/22 from 7ea8703b`: for nodes whose address starts so (8+ digits);
//! - `any from 7ea8703b…`: everything, for those nodes;
//! - `icmp`: ping (echo), for every node; `icmp from …` likewise for some.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::identity::Hash;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Proto {
    Tcp,
    Udp,
    Icmp,
    Any,
}

#[derive(Clone, Debug)]
pub struct Rule {
    proto: Proto,
    ports: (u16, u16),
    /// The leading bytes of the addresses it's for (None: every node).
    from: Option<Vec<u8>>,
    text: String,
}

impl Rule {
    pub fn parse(s: &str) -> Result<Rule, String> {
        let text = s.trim().to_ascii_lowercase();
        let (what, from) = match text.split_once(" from ") {
            Some((w, f)) => (w.trim(), Some(f.trim())),
            None => (text.as_str(), None),
        };
        let from = match from {
            Some(f) => {
                let f = f.trim_end_matches(".vork");
                if f.len() < 8 || f.len() > 32 || f.len() % 2 != 0 || !f.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(format!("{s}: after \"from\" goes a node's address, at least its first 8 digits (an even count)"));
                }
                Some(hex::decode(f).map_err(|e| e.to_string())?)
            }
            None => None,
        };
        let (proto, ports) = match what {
            "any" | "all" => (Proto::Any, (0, u16::MAX)),
            "icmp" | "ping" => (Proto::Icmp, (0, 0)),
            _ => {
                let (p, range) = what.split_once('/').ok_or(format!("{s}: a rule is tcp/PORT, udp/PORT, icmp or any (then \"from ADDRESS\" if only for some)"))?;
                let proto = match p {
                    "tcp" => Proto::Tcp,
                    "udp" => Proto::Udp,
                    _ => return Err(format!("{s}: {p} isn't tcp or udp")),
                };
                let port = |t: &str| t.trim().parse::<u16>().map_err(|_| format!("{s}: {t} isn't a port"));
                let ports = match range.split_once('-') {
                    Some((a, b)) => (port(a)?, port(b)?),
                    None => {
                        let p = port(range)?;
                        (p, p)
                    }
                };
                if ports.0 > ports.1 {
                    return Err(format!("{s}: the range runs backwards"));
                }
                (proto, ports)
            }
        };
        if proto == Proto::Any && from.is_none() {
            return Err(format!("{s}: everything for every node would leave no garden; name the node (any from ADDRESS) or open ports"));
        }
        Ok(Rule { proto, ports, from, text })
    }

    fn matches(&self, from: &Hash, proto: u8, port: u16, echo: bool) -> bool {
        if let Some(f) = &self.from {
            if !from.starts_with(f) {
                return false;
            }
        }
        match self.proto {
            Proto::Any => true,
            Proto::Icmp => proto == 58 && echo,
            Proto::Tcp => proto == 6 && (self.ports.0..=self.ports.1).contains(&port),
            Proto::Udp => proto == 17 && (self.ports.0..=self.ports.1).contains(&port),
        }
    }
}

/// How long an idle conversation this node began stays open for answers.
const FLOW_IDLE: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct Firewall {
    pub rules: Vec<Rule>,
    /// Conversations begun here, by (node, protocol, this end's port or echo id, far port).
    flows: HashMap<(Hash, u8, u16, u16), Instant>,
}

/// An IPv6 packet's protocol and its transport ports (or echo id), if it has
/// them: (protocol, source port, destination port, ICMPv6 type).
fn parts(p: &[u8]) -> Option<(u8, u16, u16, u8)> {
    if p.len() < 44 || p[0] >> 4 != 6 {
        return None;
    }
    let b = &p[40..];
    let port = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    match p[6] {
        6 | 17 => Some((p[6], port(0), port(2), 0)),
        58 if b.len() >= 8 => Some((58, port(4), 0, b[0])),
        58 => Some((58, 0, 0, b[0])),
        n => Some((n, 0, 0, 0)),
    }
}

impl Firewall {
    pub fn new(rules: &[String]) -> Result<Firewall, String> {
        Ok(Firewall { rules: rules.iter().map(|r| Rule::parse(r)).collect::<Result<_, _>>()?, flows: HashMap::new() })
    }

    /// A packet this node sends to `to`: its answers may come back.
    pub fn note_out(&mut self, to: &Hash, p: &[u8]) {
        let Some((proto, sport, dport, icmp)) = parts(p) else { return };
        let key = match proto {
            6 | 17 => (*to, proto, sport, dport),
            58 if icmp == 128 => (*to, 58, sport, 0), // echo request: (id in sport)
            _ => return,
        };
        if self.flows.len() > 8192 {
            self.flows.retain(|_, t| t.elapsed() < FLOW_IDLE);
        }
        self.flows.insert(key, Instant::now());
    }

    /// Whether a packet from `from` may reach this node's services. Err: why
    /// not, for the log.
    pub fn check_in(&mut self, from: &Hash, device: bool, p: &[u8]) -> Result<(), String> {
        if device {
            return Ok(());
        }
        let Some((proto, sport, dport, icmp)) = parts(p) else { return Err("not IPv6".into()) };
        // Answers in conversations begun here.
        let answer = match proto {
            6 | 17 => Some((*from, proto, dport, sport)),
            58 if icmp == 129 => Some((*from, 58, sport, 0)),
            _ => None,
        };
        if let Some(k) = answer {
            if let Some(t) = self.flows.get_mut(&k) {
                if t.elapsed() < FLOW_IDLE {
                    *t = Instant::now();
                    return Ok(());
                }
            }
        }
        // ICMPv6 errors (unreachable, too big, time exceeded, parameter problem) keep connections working.
        if proto == 58 && (1..=4).contains(&icmp) {
            return Ok(());
        }
        let echo = proto == 58 && icmp == 128;
        if self.rules.iter().any(|r| r.matches(from, proto, dport, echo)) {
            return Ok(());
        }
        Err(match proto {
            6 => format!("tcp/{dport}"),
            17 => format!("udp/{dport}"),
            58 if echo => "icmp (ping)".to_string(),
            58 => format!("icmp type {icmp}"),
            n => format!("protocol {n}"),
        })
    }

    pub fn rules_text(&self) -> Vec<String> {
        self.rules.iter().map(|r| r.text.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(proto: u8, sport: u16, dport: u16, icmp: u8) -> Vec<u8> {
        let mut p = vec![0u8; 48];
        p[0] = 0x60;
        p[6] = proto;
        if proto == 58 {
            p[40] = icmp;
            p[44..46].copy_from_slice(&sport.to_be_bytes());
        } else {
            p[40..42].copy_from_slice(&sport.to_be_bytes());
            p[42..44].copy_from_slice(&dport.to_be_bytes());
        }
        p
    }

    const FRIEND: Hash = [0x7e, 0xa8, 0x70, 0x3b, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    const STRANGER: Hash = [0xab; 16];

    #[test]
    fn devices_reach_everything_others_nothing_by_default() {
        let mut f = Firewall::new(&[]).unwrap();
        assert!(f.check_in(&STRANGER, true, &packet(6, 5000, 22, 0)).is_ok());
        assert_eq!(f.check_in(&STRANGER, false, &packet(6, 5000, 22, 0)), Err("tcp/22".into()));
        assert!(f.check_in(&STRANGER, false, &packet(58, 7, 0, 128)).is_err(), "ping closed");
        assert!(f.check_in(&STRANGER, false, &packet(58, 0, 0, 2)).is_ok(), "packet too big passes");
    }

    #[test]
    fn holes() {
        let mut f = Firewall::new(&["tcp/80".into(), "udp/5000-5010".into(), "tcp/22 from 7ea8703b".into(), "icmp".into()]).unwrap();
        assert!(f.check_in(&STRANGER, false, &packet(6, 4000, 80, 0)).is_ok());
        assert!(f.check_in(&STRANGER, false, &packet(17, 4000, 5005, 0)).is_ok());
        assert!(f.check_in(&STRANGER, false, &packet(17, 4000, 5011, 0)).is_err());
        assert!(f.check_in(&STRANGER, false, &packet(6, 4000, 22, 0)).is_err());
        assert!(f.check_in(&FRIEND, false, &packet(6, 4000, 22, 0)).is_ok());
        assert!(f.check_in(&STRANGER, false, &packet(58, 9, 0, 128)).is_ok());
        let mut g = Firewall::new(&["any from 7ea8703b".into()]).unwrap();
        assert!(g.check_in(&FRIEND, false, &packet(17, 1, 9999, 0)).is_ok());
        assert!(g.check_in(&STRANGER, false, &packet(17, 1, 9999, 0)).is_err());
    }

    #[test]
    fn answers_to_conversations_begun_here() {
        let mut f = Firewall::new(&[]).unwrap();
        f.note_out(&STRANGER, &packet(6, 40000, 443, 0));
        assert!(f.check_in(&STRANGER, false, &packet(6, 443, 40000, 0)).is_ok());
        assert!(f.check_in(&STRANGER, false, &packet(6, 443, 40001, 0)).is_err(), "another port isn't an answer");
        assert!(f.check_in(&FRIEND, false, &packet(6, 443, 40000, 0)).is_err(), "another node isn't either");
        f.note_out(&STRANGER, &packet(58, 77, 0, 128));
        assert!(f.check_in(&STRANGER, false, &packet(58, 77, 0, 129)).is_ok());
    }

    #[test]
    fn bad_rules_say_why() {
        assert!(Rule::parse("any").is_err());
        assert!(Rule::parse("tcp/99999").is_err());
        assert!(Rule::parse("tcp/22 from 7ea8").is_err());
        assert!(Rule::parse("sctp/5").is_err());
        assert!(Rule::parse("TCP/22 FROM 7EA8703B").is_ok());
    }
}

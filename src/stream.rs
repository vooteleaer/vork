//! Reliable, ordered delivery of messages inside a session (ADR-0022).
//!
//! A session's packets can be lost or reordered (several paths, relays that
//! drop). Requests and responses of up to 32 MiB need more: this is a small
//! sliding-window protocol over the session. Segments carry a sequence number;
//! the receiver acknowledges cumulatively plus up to 8 ranges received past
//! the gap (selective acks). The sender keeps a congestion window that
//! grows like TCP's (slow start, then additive increase) and halves on loss,
//! detected when three later segments arrived or a retransmission timer ran out.
//!
//! The byte stream carries messages, each prefixed with its length (u32).

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// Payload bytes per segment: fits the inner MTU with all headers.
pub const SEG_DATA: usize = 1180;
/// Largest message accepted.
pub const MAX_MESSAGE: usize = (32 << 20) + 1024;
/// Segments the receiver holds beyond the next expected one.
const RCV_WINDOW: u32 = 8192;
const INIT_CWND: f64 = 16.0;
const MAX_CWND: f64 = RCV_WINDOW as f64;
const MIN_RTO: Duration = Duration::from_millis(200);
const MAX_RTO: Duration = Duration::from_secs(10);
const INIT_RTO: Duration = Duration::from_secs(1);
const ACK_EVERY: u32 = 2;
const ACK_DELAY: Duration = Duration::from_millis(10);
/// Later segments that must have arrived before a missing one counts as lost.
const REORDER: u32 = 3;

#[derive(Debug, PartialEq)]
pub enum Out {
    Seg { seq: u32, data: Vec<u8> },
    /// Everything before `next`, and the ranges [start, end) after it.
    Ack { next: u32, ranges: Vec<(u32, u32)> },
}

/// Most selective-ack ranges in one acknowledgment.
pub const MAX_RANGES: usize = 8;

struct Sent {
    data: Vec<u8>,
    sent: Instant,
    retx: bool,
    sacked: bool,
    lost: bool,
}

pub struct Stream {
    pending: VecDeque<u8>,
    next_seq: u32,
    una: u32,
    inflight: BTreeMap<u32, Sent>,
    cwnd: f64,
    ssthresh: f64,
    srtt: Option<f64>,
    rttvar: f64,
    rto: Duration,
    recover: Option<u32>,
    /// Send time of the latest-sent segment acknowledged so far (RACK).
    rack: Option<Instant>,
    /// Last time an acknowledgment covered something new.
    progress: Instant,
    /// A tail probe went out since then.
    probed: bool,
    rcv_next: u32,
    ooo: BTreeMap<u32, Vec<u8>>,
    rbuf: Vec<u8>,
    unacked_rx: u32,
    ack_due: Option<Instant>,
    pub retransmits: u64,
    pub timeouts: u64,
    pub losses: u64,
}

impl Default for Stream {
    fn default() -> Self {
        Stream {
            pending: VecDeque::new(),
            next_seq: 0,
            una: 0,
            inflight: BTreeMap::new(),
            cwnd: INIT_CWND,
            ssthresh: MAX_CWND,
            srtt: None,
            rttvar: 0.0,
            rto: INIT_RTO,
            recover: None,
            rack: None,
            progress: Instant::now(),
            probed: false,
            rcv_next: 0,
            ooo: BTreeMap::new(),
            rbuf: Vec::new(),
            unacked_rx: 0,
            ack_due: None,
            retransmits: 0,
            timeouts: 0,
            losses: 0,
        }
    }
}

impl Stream {
    /// Queues a message for delivery.
    pub fn send(&mut self, msg: &[u8]) {
        self.pending.extend((msg.len() as u32).to_be_bytes());
        self.pending.extend(msg);
    }

    /// Nothing left to send or to be acknowledged.
    pub fn idle(&self) -> bool {
        self.pending.is_empty() && self.inflight.is_empty() && self.ack_due.is_none()
    }

    pub fn srtt(&self) -> Option<Duration> {
        self.srtt.map(Duration::from_secs_f64)
    }

    pub fn on_seg(&mut self, seq: u32, data: &[u8], now: Instant) {
        let fresh = seq.wrapping_sub(self.rcv_next) < RCV_WINDOW;
        if fresh && !self.ooo.contains_key(&seq) {
            self.ooo.insert(seq, data.to_vec());
            while let Some(d) = self.ooo.remove(&self.rcv_next) {
                self.rbuf.extend_from_slice(&d);
                self.rcv_next = self.rcv_next.wrapping_add(1);
            }
        }
        self.unacked_rx += 1;
        // A duplicate or a gap: tell the sender at once.
        if !fresh || !self.ooo.is_empty() || self.unacked_rx >= ACK_EVERY {
            self.ack_due = Some(now);
        } else {
            self.ack_due.get_or_insert(now + ACK_DELAY);
        }
    }

    /// Complete messages received so far. An over-long length is an error
    /// (the caller ends the session).
    pub fn messages(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::new();
        let mut at = 0;
        while self.rbuf.len() - at >= 4 {
            let n = u32::from_be_bytes(self.rbuf[at..at + 4].try_into().unwrap()) as usize;
            if n > MAX_MESSAGE {
                return Err(format!("message of {n} bytes is too long"));
            }
            if self.rbuf.len() - at - 4 < n {
                break;
            }
            out.push(self.rbuf[at + 4..at + 4 + n].to_vec());
            at += 4 + n;
        }
        self.rbuf.drain(..at);
        Ok(out)
    }

    fn rtt_sample(&mut self, rtt: Duration) {
        let r = rtt.as_secs_f64();
        match self.srtt {
            None => {
                self.srtt = Some(r);
                self.rttvar = r / 2.0;
            }
            Some(s) => {
                self.rttvar = 0.75 * self.rttvar + 0.25 * (s - r).abs();
                self.srtt = Some(0.875 * s + 0.125 * r);
            }
        }
        let rto = self.srtt.unwrap() + 4.0 * self.rttvar;
        self.rto = Duration::from_secs_f64(rto).clamp(MIN_RTO, MAX_RTO);
    }

    fn grow(&mut self, acked: usize) {
        if self.recover.is_some() {
            return;
        }
        for _ in 0..acked {
            self.cwnd += if self.cwnd < self.ssthresh { 1.0 } else { 1.0 / self.cwnd };
        }
        self.cwnd = self.cwnd.min(MAX_CWND);
    }

    fn on_loss(&mut self) {
        if self.recover.is_none() {
            self.losses += 1;
            self.ssthresh = (self.cwnd / 2.0).max(2.0);
            self.cwnd = self.ssthresh;
            self.recover = Some(self.next_seq);
        }
    }

    pub fn on_ack(&mut self, next: u32, ranges: &[(u32, u32)], now: Instant) {
        let mut acked = 0;
        // Cumulative: everything before `next`.
        if next.wrapping_sub(self.una) <= self.next_seq.wrapping_sub(self.una) {
            while self.una != next {
                if let Some(s) = self.inflight.remove(&self.una) {
                    if !s.retx && !s.sacked {
                        self.rtt_sample(now - s.sent);
                    }
                    if !s.sacked {
                        acked += 1;
                        self.rack = Some(self.rack.map_or(s.sent, |r| r.max(s.sent)));
                    }
                }
                self.una = self.una.wrapping_add(1);
            }
        }
        // Selective: the ranges past the gap.
        let mut highest = None;
        for &(start, end) in ranges.iter().take(MAX_RANGES) {
            if end.wrapping_sub(start) > RCV_WINDOW {
                continue;
            }
            let mut seq = start;
            while seq != end {
                if let Some(s) = self.inflight.get_mut(&seq) {
                    if !s.sacked {
                        s.sacked = true;
                        s.lost = false;
                        acked += 1;
                        let (sent, retx) = (s.sent, s.retx);
                        self.rack = Some(self.rack.map_or(sent, |r| r.max(sent)));
                        if !retx {
                            self.rtt_sample(now - sent);
                        }
                    }
                }
                highest = Some(seq);
                seq = seq.wrapping_add(1);
            }
        }
        if let Some(r) = self.recover {
            if self.una.wrapping_sub(r) < u32::MAX / 2 {
                self.recover = None;
            }
        }
        self.grow(acked);
        if acked > 0 {
            self.progress = now;
            self.probed = false;
            // Acknowledgments flow again: undo the timer's backoff.
            if let Some(srtt) = self.srtt {
                self.rto = Duration::from_secs_f64(srtt + 4.0 * self.rttvar).clamp(MIN_RTO, MAX_RTO);
            }
        }
        // A missing segment is lost when REORDER later segments arrived, or
        // when one sent later than it (a retransmission too) was acknowledged.
        let reo = Duration::from_secs_f64(self.srtt.unwrap_or(0.0) / 4.0).max(Duration::from_millis(1));
        let mut lost = false;
        for (seq, s) in self.inflight.iter_mut() {
            if s.sacked || s.lost {
                continue;
            }
            let by_count = !s.retx && highest.is_some_and(|h: u32| h.wrapping_sub(*seq) >= REORDER && h.wrapping_sub(*seq) < u32::MAX / 2);
            let by_time = self.rack.is_some_and(|r| s.sent + reo < r);
            if by_count || by_time {
                s.lost = true;
                lost = true;
            }
        }
        if lost {
            self.on_loss();
        }
    }

    pub fn poll(&mut self, now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        // Retransmission timer: segments are out and no acknowledgment
        // covered anything new for a whole timeout.
        let waiting = self.inflight.values().any(|s| !s.sacked && !s.lost);
        if waiting && now.duration_since(self.progress) >= self.rto {
            for s in self.inflight.values_mut() {
                if !s.sacked && now.duration_since(s.sent) >= self.rto {
                    s.lost = true;
                }
            }
            self.progress = now;
            self.timeouts += 1;
            self.ssthresh = (self.cwnd / 2.0).max(2.0);
            self.cwnd = 2.0;
            self.recover = Some(self.next_seq);
            self.rto = (self.rto * 2).min(MAX_RTO);
        }
        // Tail probe: nothing acknowledged for two round trips, maybe because
        // the last acknowledgment was lost. Resend the newest segment to draw one.
        let pto = Duration::from_secs_f64(2.0 * self.srtt.unwrap_or(0.1)).max(Duration::from_millis(10));
        if waiting && !self.probed && now.duration_since(self.progress) >= pto {
            if let Some((seq, s)) = self.inflight.iter_mut().rev().find(|(_, s)| !s.sacked) {
                s.retx = true;
                s.sent = now;
                self.retransmits += 1;
                out.push(Out::Seg { seq: *seq, data: s.data.clone() });
            }
            self.probed = true;
        }
        let outstanding = self.inflight.values().filter(|s| !s.sacked && !s.lost).count();
        let mut budget = (self.cwnd as usize).saturating_sub(outstanding);
        for (seq, s) in self.inflight.iter_mut() {
            if budget == 0 {
                break;
            }
            if s.lost {
                s.lost = false;
                s.retx = true;
                s.sent = now;
                self.retransmits += 1;
                out.push(Out::Seg { seq: *seq, data: s.data.clone() });
                budget -= 1;
            }
        }
        while budget > 0 && !self.pending.is_empty() && self.next_seq.wrapping_sub(self.una) < RCV_WINDOW {
            let n = self.pending.len().min(SEG_DATA);
            let data: Vec<u8> = self.pending.drain(..n).collect();
            let seq = self.next_seq;
            self.next_seq = self.next_seq.wrapping_add(1);
            self.inflight.insert(seq, Sent { data: data.clone(), sent: now, retx: false, sacked: false, lost: false });
            out.push(Out::Seg { seq, data });
            budget -= 1;
        }
        if self.ack_due.is_some_and(|t| t <= now) {
            let mut ranges: Vec<(u32, u32)> = Vec::new();
            for &seq in self.ooo.keys() {
                if let Some((_, end)) = ranges.last_mut() {
                    if *end == seq {
                        *end = seq.wrapping_add(1);
                        continue;
                    }
                }
                if ranges.len() == MAX_RANGES {
                    break;
                }
                ranges.push((seq, seq.wrapping_add(1)));
            }
            out.push(Out::Ack { next: self.rcv_next, ranges });
            self.ack_due = None;
            self.unacked_rx = 0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two streams over a link that drops every `drop_every`-th segment and
    /// delays everything by one step.
    fn transfer(sizes: &[usize], drop_every: usize) -> (Vec<Vec<u8>>, u64) {
        let (mut a, mut b) = (Stream::default(), Stream::default());
        let msgs: Vec<Vec<u8>> = sizes.iter().enumerate().map(|(i, n)| (0..*n).map(|j| (i * 7 + j) as u8).collect()).collect();
        for m in &msgs {
            a.send(m);
        }
        let mut now = Instant::now();
        let mut got = Vec::new();
        let mut count = 0usize;
        for _ in 0..200_000 {
            now += Duration::from_millis(5);
            for o in a.poll(now) {
                if let Out::Seg { seq, data } = o {
                    count += 1;
                    if drop_every == 0 || count % drop_every != 0 {
                        b.on_seg(seq, &data, now);
                    }
                }
            }
            for o in b.poll(now) {
                if let Out::Ack { next, ranges } = o {
                    a.on_ack(next, &ranges, now);
                }
            }
            got.extend(b.messages().unwrap());
            if got.len() == msgs.len() && a.idle() {
                break;
            }
        }
        assert_eq!(got, msgs);
        (got, a.retransmits)
    }

    #[test]
    fn delivers_in_order_without_loss() {
        let (_, retx) = transfer(&[0, 1, 1180, 1181, 100_000, 3], 0);
        assert_eq!(retx, 0);
    }

    #[test]
    fn recovers_from_loss() {
        let (_, retx) = transfer(&[2_000_000, 10, 50_000], 7);
        assert!(retx > 0);
    }

    #[test]
    fn heavy_loss_still_completes() {
        transfer(&[300_000], 2);
    }

    #[test]
    fn duplicates_and_reordering_are_harmless() {
        let mut b = Stream::default();
        let now = Instant::now();
        let mut a = Stream::default();
        a.send(&[9u8; 3000]);
        let segs: Vec<(u32, Vec<u8>)> = a.poll(now).into_iter().filter_map(|o| if let Out::Seg { seq, data } = o { Some((seq, data)) } else { None }).collect();
        for (seq, d) in segs.iter().rev().chain(segs.iter()) {
            b.on_seg(*seq, d, now);
        }
        assert_eq!(b.messages().unwrap(), vec![vec![9u8; 3000]]);
    }

    #[test]
    fn over_long_message_is_an_error() {
        let mut b = Stream::default();
        b.on_seg(0, &u32::MAX.to_be_bytes(), Instant::now());
        assert!(b.messages().is_err());
    }
}

#[cfg(test)]
mod sim {
    use super::*;

    /// Random loss both ways, a one-step delay of 1 ms: returns simulated seconds.
    pub fn lossy(bytes: usize, loss: f64, seed: u64) -> f64 {
        let mut rng = seed;
        let mut rand = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng % 1_000_000) as f64 / 1_000_000.0
        };
        let (mut a, mut b) = (Stream::default(), Stream::default());
        a.send(&vec![7u8; bytes]);
        let start = Instant::now();
        let mut now = start;
        let mut to_b: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut to_a: Vec<(u32, Vec<(u32, u32)>)> = Vec::new();
        loop {
            now += Duration::from_millis(1);
            for (seq, d) in std::mem::take(&mut to_b) {
                b.on_seg(seq, &d, now);
            }
            for (n, r) in std::mem::take(&mut to_a) {
                a.on_ack(n, &r, now);
            }
            for o in a.poll(now) {
                if let Out::Seg { seq, data } = o {
                    if rand() >= loss {
                        to_b.push((seq, data));
                    }
                }
            }
            for o in b.poll(now) {
                if let Out::Ack { next, ranges } = o {
                    if rand() >= loss {
                        to_a.push((next, ranges));
                    }
                }
            }
            if !b.messages().unwrap().is_empty() {
                println!("  timeouts {} loss events {} resent {} cwnd {:.1} srtt {:?}", a.timeouts, a.losses, a.retransmits, a.cwnd, a.srtt());
                return (now - start).as_secs_f64();
            }
            assert!(now - start < Duration::from_secs(600), "stuck: una {} next {} cwnd {} rto {:?}", a.una, a.next_seq, a.cwnd, a.rto);
        }
    }

    #[test]
    fn lossy_throughput() {
        for loss in [0.0, 0.01, 0.05, 0.1] {
            let t = lossy(4 << 20, loss, 12345);
            println!("loss {loss}: 4 MiB in {t:.2} s simulated ({:.1} Mbit/s)", 32.0 / t);
        }
    }
}

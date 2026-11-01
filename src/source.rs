//! Transport sources and the `MoldUDP64` / bare-ITCH feed adapters.
//!
//! Two layers live here:
//!
//! * [`PacketSource`] — a raw byte transport. It knows nothing about ITCH; it
//!   just hands back framed packets or datagrams.
//! * [`FeedSource`] — a stream of ITCH messages, optionally annotated with a
//!   [`FeedEvent`] describing a session change, a sequence gap, a truncated
//!   packet, or the end of a session.
//!
//! [`MoldUdp64Source`] sits on top of a packet source and applies `MoldUDP64`
//! framing and sequencing. [`BareItchSource`] sits on top of a 2-byte
//! length-prefixed file and yields bare ITCH messages, which is how Nasdaq
//! ships historical captures. Because a bare file carries no sequence numbers,
//! it can never report a gap — only `MoldUDP64` can.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, ToSocketAddrs, UdpSocket};
use std::path::Path;

// ---------------------------------------------------------------------------
// Raw packet transport
// ---------------------------------------------------------------------------

pub trait PacketSource {
    /// Reads the next packet's bytes into `buf`, overwriting its contents.
    /// Returns `Ok(Some(n))` where the first `n` bytes of `buf` are the
    /// packet, or `Ok(None)` if the source is exhausted (end of file / no more
    /// data expected). Bytes in `buf` beyond index `n` are unspecified and
    /// must not be relied upon.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying transport fails while producing the
    /// next packet (e.g. an I/O error reading the file, or a socket error on
    /// `recv_from`).
    fn next_packet(&mut self, buf: &mut Vec<u8>) -> std::io::Result<Option<usize>>;
}

/// Width of the length prefix that frames packets in a file.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum LengthPrefix {
    /// Two big-endian bytes. Nasdaq historical ITCH captures use this.
    U16,
    /// Four big-endian bytes. The project's own fixture format.
    #[default]
    U32,
}

impl LengthPrefix {
    #[must_use]
    pub const fn width(self) -> usize {
        match self {
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }

    const fn decode(self, bytes: &[u8]) -> usize {
        match self {
            Self::U16 => u16::from_be_bytes([bytes[0], bytes[1]]) as usize,
            Self::U32 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
        }
    }

    fn encode(self, length: usize) -> Vec<u8> {
        match self {
            Self::U16 => {
                let narrowed = u16::try_from(length)
                    .expect("packet length exceeds u16::MAX for a U16 length prefix");
                narrowed.to_be_bytes().to_vec()
            }
            Self::U32 => {
                let narrowed = u32::try_from(length)
                    .expect("packet length exceeds u32::MAX for a U32 length prefix");
                narrowed.to_be_bytes().to_vec()
            }
        }
    }
}

pub struct FilePacketSource {
    reader: BufReader<File>,
    prefix: LengthPrefix,
}

impl FilePacketSource {
    /// Opens a file framed with 4-byte big-endian length prefixes.
    ///
    /// # Errors
    ///
    /// Returns an error if the file at `path` cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::open_with_prefix(path, LengthPrefix::U32)
    }

    /// Opens a file framed with the given prefix width.
    ///
    /// Use [`LengthPrefix::U16`] for Nasdaq historical TotalView-ITCH captures.
    ///
    /// # Errors
    ///
    /// Returns an error if the file at `path` cannot be opened.
    pub fn open_with_prefix(path: impl AsRef<Path>, prefix: LengthPrefix) -> std::io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            reader: BufReader::new(file),
            prefix,
        })
    }
}

impl PacketSource for FilePacketSource {
    fn next_packet(&mut self, buf: &mut Vec<u8>) -> std::io::Result<Option<usize>> {
        let mut length_buf = [0u8; 4];
        let width = self.prefix.width();
        let prefix = &mut length_buf[..width];

        // A clean end of stream is only clean at a frame boundary. A partial
        // prefix means the file was cut mid-frame, which is an error.
        let mut read = 0;
        while read < width {
            match self.reader.read(&mut prefix[read..])? {
                0 if read == 0 => return Ok(None),
                0 => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "file ended part-way through a length prefix",
                    ));
                }
                n => read += n,
            }
        }

        let packet_length = self.prefix.decode(prefix);
        buf.resize(packet_length, 0);
        self.reader.read_exact(buf)?;
        Ok(Some(packet_length))
    }
}

const MAX_UDP_DATAGRAM_SIZE: usize = 65_536;

pub struct UdpPacketSource {
    socket: UdpSocket,
}

impl UdpPacketSource {
    /// # Errors
    ///
    /// Returns an error if the socket cannot be bound to `port`, or if joining
    /// the multicast group `multicast_addr` on `interface` fails.
    pub fn join_multicast(
        multicast_addr: Ipv4Addr,
        port: u16,
        interface: Ipv4Addr,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?;
        socket.join_multicast_v4(&multicast_addr, &interface)?;
        Ok(Self { socket })
    }

    /// # Errors
    ///
    /// Returns an error if the socket cannot be bound to `addr`.
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self {
            socket: UdpSocket::bind(addr)?,
        })
    }
}

impl PacketSource for UdpPacketSource {
    fn next_packet(&mut self, buf: &mut Vec<u8>) -> std::io::Result<Option<usize>> {
        buf.resize(MAX_UDP_DATAGRAM_SIZE, 0);
        let (received, _origin) = self.socket.recv_from(buf)?;
        buf.truncate(received);
        Ok(Some(received))
    }
}

// ---------------------------------------------------------------------------
// Feed layer
// ---------------------------------------------------------------------------

/// Something that happened to the feed, independent of any single message.
///
/// Events are delivered *before* the message they precede, so a consumer can
/// refuse to process the message if the feed integrity says it should not.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FeedEvent {
    /// The feed moved to a different (or the first) session.
    NewSession { previous: Option<[u8; 10]> },
    /// A `MoldUDP64` gap: messages `start .. start + count` were never seen.
    /// The session id is included so a retransmission request can be built
    /// without asking the consumer to track it separately.
    Gap {
        session: [u8; 10],
        start: u64,
        count: u64,
    },
    /// A packet declared more messages than its body actually contained.
    TruncatedPacket,
    /// The `MoldUDP64` end-of-session packet arrived. No further messages
    /// follow from this source.
    EndOfSession,
}

/// One item from a [`FeedSource`]: an event, a message, or both.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Feed<'a> {
    /// The message bytes, if this item carries one.
    pub message: Option<&'a [u8]>,
    /// The event, if this item carries one.
    pub event: Option<FeedEvent>,
}

impl<'a> Feed<'a> {
    const fn event(event: FeedEvent) -> Self {
        Self {
            message: None,
            event: Some(event),
        }
    }

    const fn message(bytes: &'a [u8], event: Option<FeedEvent>) -> Self {
        Self {
            message: Some(bytes),
            event,
        }
    }
}

/// Counters describing how healthy the feed has been.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct FeedStats {
    /// Packets (or, for a bare source, messages) pulled from the transport.
    pub packets: u64,
    /// ITCH messages yielded.
    pub messages: u64,
    /// Packets that declared more messages than their body held.
    pub truncated_packets: u64,
    /// Sequence gaps observed.
    pub gaps: u64,
    /// Packets rejected as stale or duplicate.
    pub stale_packets: u64,
    /// Sessions observed.
    pub sessions: u64,
    /// Whether an end-of-session marker has been seen.
    pub ended: bool,
}

pub trait FeedSource {
    /// Yields the next item, or `None` when the feed is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying transport fails.
    fn next(&mut self) -> std::io::Result<Option<Feed<'_>>>;

    /// A snapshot of the feed's health counters.
    fn stats(&self) -> FeedStats;
}

// ---------------------------------------------------------------------------
// MoldUDP64
// ---------------------------------------------------------------------------

/// What to do when a `MoldUDP64` sequence gap is detected.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum GapPolicy {
    /// Stop advancing and deliver nothing until the missing messages are
    /// retransmitted, or a new session begins. The safe default for anything
    /// that maintains state, such as an order book.
    #[default]
    Halt,
    /// Advance past the gap and keep delivering. Useful for consumers that
    /// tolerate loss, such as a statistics-only listener.
    Skip,
}

pub struct MoldUdp64Source<S> {
    source: S,
    session: crate::Session,
    buf: Vec<u8>,
    /// Offset into `buf` of the next message block.
    cursor: usize,
    /// Messages left to yield from the current packet.
    remaining: u16,
    pending_event: Option<FeedEvent>,
    stats: FeedStats,
    ended: bool,
}

impl<S: PacketSource> MoldUdp64Source<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        Self::with_policy(source, GapPolicy::default())
    }

    #[must_use]
    pub fn with_policy(source: S, policy: GapPolicy) -> Self {
        Self {
            source,
            session: crate::Session::with_policy(policy),
            buf: Vec::new(),
            cursor: 0,
            remaining: 0,
            pending_event: None,
            stats: FeedStats::default(),
            ended: false,
        }
    }

    /// The inner session tracker, exposed for inspection in tests and tooling.
    #[must_use]
    pub const fn session(&self) -> &crate::Session {
        &self.session
    }

    /// Abandons the rest of the current packet as truncated.
    const fn truncate(&mut self) {
        self.remaining = 0;
        self.stats.truncated_packets += 1;
        self.pending_event = Some(FeedEvent::TruncatedPacket);
    }

    fn read_packet(&mut self) -> std::io::Result<()> {
        let Some(packet_length) = self.source.next_packet(&mut self.buf)? else {
            return Ok(());
        };
        self.stats.packets += 1;

        let Some((header, _body)) = crate::parse_header(&self.buf[..packet_length]) else {
            self.truncate();
            return Ok(());
        };

        self.cursor = crate::MOLD_UDP64_HEADER_LEN;
        let status = self.session.on_packet(&header);

        let delivery_count = match status {
            crate::PacketStatus::NewSession { previous } => {
                self.stats.sessions += 1;
                self.pending_event = Some(FeedEvent::NewSession { previous });
                header.message_count
            }
            crate::PacketStatus::InOrder => header.message_count,
            crate::PacketStatus::Gap { start, count } => {
                self.stats.gaps += 1;
                self.pending_event = Some(FeedEvent::Gap {
                    session: header.session,
                    start,
                    count,
                });
                // Under `Halt` the session reports itself desynced and we drop
                // this packet; under `Skip` we deliver it as usual.
                if self.session.is_desynced() {
                    0
                } else {
                    header.message_count
                }
            }
            crate::PacketStatus::StaleOrDuplicate => {
                self.stats.stale_packets += 1;
                0
            }
            crate::PacketStatus::EndOfSession => {
                self.pending_event = Some(FeedEvent::EndOfSession);
                self.ended = true;
                self.stats.ended = true;
                0
            }
        };

        self.remaining = if delivery_count == 0xFFFF {
            0
        } else {
            delivery_count
        };
        Ok(())
    }
}

impl<S: PacketSource> FeedSource for MoldUdp64Source<S> {
    fn next(&mut self) -> std::io::Result<Option<Feed<'_>>> {
        loop {
            if self.remaining > 0 {
                let Some(prefix_end) = self.cursor.checked_add(2) else {
                    self.truncate();
                    continue;
                };
                if prefix_end > self.buf.len() {
                    self.truncate();
                    continue;
                }
                let message_len = usize::from(u16::from_be_bytes([
                    self.buf[self.cursor],
                    self.buf[self.cursor + 1],
                ]));
                let message_end = prefix_end + message_len;
                if message_end > self.buf.len() {
                    self.truncate();
                    continue;
                }

                self.cursor = message_end;
                self.remaining -= 1;
                self.stats.messages += 1;
                let event = self.pending_event.take();
                return Ok(Some(Feed::message(
                    &self.buf[prefix_end..message_end],
                    event,
                )));
            }

            if let Some(event) = self.pending_event.take() {
                return Ok(Some(Feed::event(event)));
            }

            if self.ended {
                return Ok(None);
            }

            let packets_before = self.stats.packets;
            self.read_packet()?;
            if self.stats.packets == packets_before {
                // Underlying source is exhausted.
                return Ok(None);
            }
        }
    }

    fn stats(&self) -> FeedStats {
        self.stats
    }
}

// ---------------------------------------------------------------------------
// Bare ITCH (historical files)
// ---------------------------------------------------------------------------

/// Yields bare ITCH messages from a length-prefixed file.
///
/// Nasdaq historical TotalView-ITCH captures are a sequence of 2-byte
/// big-endian length prefixes each followed by one message, with no
/// `MoldUDP64` header, session id, sequence number, or end-of-session packet.
/// That means a bare source can never detect message loss: there is nothing to
/// detect it *with*. Treat a local file as complete by construction.
pub struct BareItchSource<S> {
    source: S,
    buf: Vec<u8>,
    stats: FeedStats,
}

impl<S: PacketSource> BareItchSource<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        Self {
            source,
            buf: Vec::new(),
            stats: FeedStats::default(),
        }
    }
}

impl BareItchSource<FilePacketSource> {
    /// Convenience constructor for a Nasdaq-format ITCH capture file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file at `path` cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self::new(FilePacketSource::open_with_prefix(
            path,
            LengthPrefix::U16,
        )?))
    }
}

impl<S: PacketSource> FeedSource for BareItchSource<S> {
    fn next(&mut self) -> std::io::Result<Option<Feed<'_>>> {
        let Some(length) = self.source.next_packet(&mut self.buf)? else {
            return Ok(None);
        };
        self.stats.packets += 1;
        self.stats.messages += 1;
        Ok(Some(Feed::message(&self.buf[..length], None)))
    }

    fn stats(&self) -> FeedStats {
        self.stats
    }
}

// ---------------------------------------------------------------------------
// Retransmission
// ---------------------------------------------------------------------------

/// Builds a `MoldUDP64` retransmission request.
///
/// The re-request message is three fields: the 10-byte session, the first
/// sequence number wanted, and how many packets to resend. The reply packets
/// are byte-identical to normal packets, so they need no special parsing.
#[must_use]
pub fn build_retransmission_request(
    session: [u8; 10],
    first_sequence_number: u64,
    count: u64,
) -> Vec<u8> {
    // `unwrap_or` rather than `expect`: the clamp above makes this infallible,
    // and a panic-free builder is worth more than the asserted invariant here.
    let count = u16::try_from(count).unwrap_or(u16::MAX);
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&session);
    buf.extend_from_slice(&first_sequence_number.to_be_bytes());
    buf.extend_from_slice(&count.to_be_bytes());
    buf
}

/// Sends retransmission requests to a `MoldUDP64` re-request endpoint.
pub struct Retransmitter {
    socket: UdpSocket,
    target: std::net::SocketAddrV4,
}

impl Retransmitter {
    /// # Errors
    ///
    /// Returns an error if a local socket cannot be bound.
    pub fn connect(target: std::net::SocketAddrV4) -> std::io::Result<Self> {
        Ok(Self {
            socket: UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?,
            target,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the request datagram cannot be sent.
    pub fn request(
        &mut self,
        session: [u8; 10],
        first_sequence_number: u64,
        count: u64,
    ) -> std::io::Result<()> {
        let request = build_retransmission_request(session, first_sequence_number, count);
        self.socket.send_to(&request, self.target)?;
        Ok(())
    }
}

/// Convenience wrapper that turns a [`FeedEvent::Gap`] into a request.
///
/// # Errors
///
/// Returns an error if the request datagram cannot be sent.
pub fn request_missing(
    event: &FeedEvent,
    retransmitter: &mut Retransmitter,
) -> std::io::Result<()> {
    if let FeedEvent::Gap {
        session,
        start,
        count,
    } = *event
    {
        // `+ 1` because the packet that revealed the gap was itself dropped
        // under `GapPolicy::Halt`, so it is part of what must be replayed.
        retransmitter.request(session, start, count + 1)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fixture writing
// ---------------------------------------------------------------------------

/// # Errors
///
/// Returns an error if `path` cannot be created, or if writing any packet's
/// length prefix or bytes fails.
///
/// # Panics
///
/// Panics if any packet's length does not fit the chosen prefix width.
pub fn write_packets_to_file_with_prefix(
    path: impl AsRef<Path>,
    packets: &[Vec<u8>],
    prefix: LengthPrefix,
) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for packet in packets {
        writer.write_all(&prefix.encode(packet.len()))?;
        writer.write_all(packet)?;
    }

    writer.flush()
}

/// Writes packets using the project fixture format (4-byte prefixes).
///
/// # Errors
///
/// Returns an error if `path` cannot be created, or if writing fails.
///
/// # Panics
///
/// Panics if any packet's length exceeds `u32::MAX` bytes.
pub fn write_packets_to_file(path: impl AsRef<Path>, packets: &[Vec<u8>]) -> std::io::Result<()> {
    write_packets_to_file_with_prefix(path, packets, LengthPrefix::U32)
}

// ---------------------------------------------------------------------------
// Tests
// </Generated by Claude.ai>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_end_of_session_packet, build_packet};
    use std::io::Write;

    struct TempFileGuard(std::path::PathBuf);

    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("itch_parser_{tag}_{}.bin", std::process::id()))
    }

    fn write_fixture(tag: &str, packets: &[Vec<u8>], prefix: LengthPrefix) -> TempFileGuard {
        let path = temp_path(tag);
        write_packets_to_file_with_prefix(&path, packets, prefix).expect("should write fixture");
        TempFileGuard(path)
    }

    fn collect(mut source: impl FeedSource) -> (Vec<Vec<u8>>, Vec<FeedEvent>, FeedStats) {
        let mut messages = Vec::new();
        let mut events = Vec::new();
        while let Some(feed) = source.next().expect("should read without I/O error") {
            if let Some(event) = feed.event {
                events.push(event);
            }
            if let Some(message) = feed.message {
                messages.push(message.to_vec());
            }
        }
        (messages, events, source.stats())
    }

    const SESSION: [u8; 10] = *b"SESSION001";

    // ---- FilePacketSource ----

    #[test]
    fn file_packet_source_round_trips_multiple_packets_in_order() {
        let original_packets = vec![
            build_packet(SESSION, 1, &[b"MSG_A", b"MSG_B"]),
            build_packet(SESSION, 3, &[b"MSG_C"]),
            build_packet(SESSION, 4, &[]), // heartbeat
        ];
        let guard = write_fixture("u32_round_trip", &original_packets, LengthPrefix::U32);

        let mut source = FilePacketSource::open(&guard.0).expect("should open");
        let mut buf = Vec::new();

        for expected in &original_packets {
            let received = source
                .next_packet(&mut buf)
                .expect("should read")
                .expect("should yield a packet");
            assert_eq!(&buf[..received], expected.as_slice());
        }
        assert_eq!(source.next_packet(&mut buf).expect("should read"), None);
    }

    #[test]
    fn file_packet_source_honours_a_u16_prefix() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"MSG_A"]),
            build_packet(SESSION, 2, &[b"MSG_B", b"MSG_C"]),
        ];
        let guard = write_fixture("u16_prefix", &packets, LengthPrefix::U16);

        let mut source =
            FilePacketSource::open_with_prefix(&guard.0, LengthPrefix::U16).expect("should open");
        let mut buf = Vec::new();
        for expected in &packets {
            let received = source
                .next_packet(&mut buf)
                .expect("should read")
                .expect("should yield a packet");
            assert_eq!(&buf[..received], expected.as_slice());
        }
        assert_eq!(source.next_packet(&mut buf).expect("should read"), None);
    }

    #[test]
    fn file_packet_source_errors_on_a_partial_length_prefix() {
        let path = temp_path("partial_prefix");
        let mut file = std::fs::File::create(&path).expect("should create");
        file.write_all(&[0x00]).expect("should write one byte");
        drop(file);
        let _guard = TempFileGuard(path.clone());

        let mut source = FilePacketSource::open(&path).expect("should open");
        let mut buf = Vec::new();
        let error = source
            .next_packet(&mut buf)
            .expect_err("a truncated prefix is an error, not a clean EOF");
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // ---- BareItchSource ----

    #[test]
    fn bare_itch_source_yields_messages_without_events_or_gaps() {
        let messages: Vec<Vec<u8>> = vec![
            b"SYS_EVENT_A".to_vec(),
            b"SYS_EVENT_B".to_vec(),
            b"SYS_EVENT_C".to_vec(),
        ];
        let guard = write_fixture("bare_itch", &messages, LengthPrefix::U16);

        let source = BareItchSource::open(&guard.0).expect("should open");
        let (seen, events, stats) = collect(source);

        assert_eq!(seen, messages);
        assert!(events.is_empty(), "a bare file has no sequencing to report");
        assert_eq!(stats.messages, 3);
        assert!(!stats.ended);
    }

    // ---- MoldUdp64Source ----

    #[test]
    fn mold_source_delivers_messages_and_reports_the_new_session() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"A"]),
            build_packet(SESSION, 2, &[b"B", b"C"]),
        ];
        let guard = write_fixture("mold_new_session", &packets, LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, events, stats) = collect(source);

        assert_eq!(messages, vec![b"A".to_vec(), b"B".to_vec(), b"C".to_vec()]);
        assert_eq!(events, vec![FeedEvent::NewSession { previous: None }]);
        assert_eq!(stats.messages, 3);
        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.gaps, 0);
    }

    #[test]
    fn mold_source_halts_on_a_gap_and_delivers_nothing_past_it() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"A"]), // expects 2
            build_packet(SESSION, 5, &[b"E"]), // gap of 3, dropped under Halt
            build_packet(SESSION, 6, &[b"F"]), // still desynced, dropped
        ];
        let guard = write_fixture("mold_gap_halt", &packets, LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, events, stats) = collect(source);

        assert_eq!(
            messages,
            vec![b"A".to_vec()],
            "nothing past the gap is delivered"
        );
        assert!(events.contains(&FeedEvent::Gap {
            session: SESSION,
            start: 2,
            count: 3,
        }));
        assert_eq!(
            stats.gaps, 2,
            "each out-of-order packet re-reports the open gap"
        );
    }

    #[test]
    fn mold_source_resumes_delivery_once_the_gap_is_filled() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"A"]), // expects 2
            build_packet(SESSION, 4, &[b"D"]), // gap: 2..4
            build_packet(SESSION, 2, &[b"B"]), // retransmitted, closes the gap
            build_packet(SESSION, 3, &[b"C"]), // retransmitted
            build_packet(SESSION, 4, &[b"D"]), // the retransmitted triggering packet
            build_packet(SESSION, 5, &[b"E"]), // live again
        ];
        let guard = write_fixture("mold_gap_fill", &packets, LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, _, stats) = collect(source);

        assert_eq!(
            messages,
            [
                b"A".to_vec(),
                b"B".to_vec(),
                b"C".to_vec(),
                b"D".to_vec(),
                b"E".to_vec()
            ]
        );
        assert_eq!(stats.gaps, 1);
    }

    #[test]
    fn mold_source_skip_policy_keeps_delivering_past_a_gap() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"A"]),
            build_packet(SESSION, 5, &[b"E"]),
            build_packet(SESSION, 6, &[b"F"]),
        ];
        let guard = write_fixture("mold_gap_skip", &packets, LengthPrefix::U32);
        let source = MoldUdp64Source::with_policy(
            FilePacketSource::open(&guard.0).unwrap(),
            GapPolicy::Skip,
        );

        let (messages, events, _) = collect(source);

        assert_eq!(messages, vec![b"A".to_vec(), b"E".to_vec(), b"F".to_vec()]);
        assert!(events.contains(&FeedEvent::Gap {
            session: SESSION,
            start: 2,
            count: 3,
        }));
    }

    #[test]
    fn mold_source_drops_stale_and_duplicate_packets() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"A"]),
            build_packet(SESSION, 1, &[b"A"]), // duplicate
            build_packet(SESSION, 2, &[b"B"]),
        ];
        let guard = write_fixture("mold_stale", &packets, LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, _, stats) = collect(source);

        assert_eq!(messages, vec![b"A".to_vec(), b"B".to_vec()]);
        assert_eq!(stats.stale_packets, 1);
    }

    #[test]
    fn mold_source_stops_at_end_of_session() {
        let packets = vec![
            build_packet(SESSION, 1, &[b"A"]),
            build_end_of_session_packet(SESSION, 2),
            build_packet(SESSION, 3, &[b"NEVER"]),
        ];
        let guard = write_fixture("mold_eos", &packets, LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, events, stats) = collect(source);

        assert_eq!(
            messages,
            vec![b"A".to_vec()],
            "nothing after EOS is delivered"
        );
        assert!(events.contains(&FeedEvent::EndOfSession));
        assert!(stats.ended);
    }

    #[test]
    fn mold_source_reports_a_truncated_packet() {
        let mut packet = build_packet(SESSION, 1, &[b"ABC", b"DEF"]);
        packet.truncate(packet.len() - 2); // cut the second message's payload
        let guard = write_fixture("mold_truncated", &[packet], LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, events, stats) = collect(source);

        assert_eq!(messages, vec![b"ABC".to_vec()]);
        assert!(events.contains(&FeedEvent::TruncatedPacket));
        assert_eq!(stats.truncated_packets, 1);
    }

    #[test]
    fn mold_source_reports_a_packet_shorter_than_the_header() {
        let guard = write_fixture("mold_short_header", &[vec![0u8; 10]], LengthPrefix::U32);
        let source = MoldUdp64Source::new(FilePacketSource::open(&guard.0).unwrap());

        let (messages, events, stats) = collect(source);

        assert!(messages.is_empty());
        assert!(events.contains(&FeedEvent::TruncatedPacket));
        assert_eq!(stats.truncated_packets, 1);
    }

    // ---- Retransmission ----

    #[test]
    fn retransmission_request_has_the_moldudp64_wire_layout() {
        let request = build_retransmission_request(SESSION, 0x0102_0304_0506_0708, 3);
        assert_eq!(request.len(), 20);
        assert_eq!(&request[..10], b"SESSION001");
        assert_eq!(&request[10..18], &0x0102_0304_0506_0708u64.to_be_bytes());
        assert_eq!(&request[18..20], &3u16.to_be_bytes());
    }

    #[test]
    fn retransmission_count_is_clamped_to_u16_max() {
        let request = build_retransmission_request(SESSION, 1, u64::MAX);
        assert_eq!(&request[18..20], &u16::MAX.to_be_bytes());
    }
}
// </Generated by Claude.ai>

//! A decoder for Nasdaq's TotalView-ITCH 5.0 feed, carried over `MoldUDP64`.
//!
//! The crate is split into three layers:
//!
//! * [`source`] — transports ([`PacketSource`](source::PacketSource)) and feed
//!   adapters ([`FeedSource`](source::FeedSource)) that turn bytes into a stream
//!   of ITCH messages plus integrity events.
//! * [`Session`] — `MoldUDP64` sequencing: session tracking, gap detection, and
//!   stale/duplicate rejection.
//! * [`itch`] — pure, allocation-free decoding of every ITCH 5.0 message type.
//!
//! # Integrity model
//!
//! A `MoldUDP64` stream can lose packets, so the feed adapters report gaps and
//! (by default) refuse to deliver anything past one. [`GapPolicy::Halt`] is the
//! correct choice for any consumer that maintains state — an order book fed
//! messages after an undetected hole will silently diverge. [`GapPolicy::Skip`]
//! is available for consumers that tolerate loss.
//!
//! A bare ITCH file has no sequence numbers, so a file source can never detect
//! loss; it is complete by construction.

// The README is compiled as part of the crate docs so its examples cannot rot.
#![doc = include_str!("../README.md")]

pub mod itch;
pub mod reference;
pub mod source;

use std::ops::ControlFlow;

use source::{FeedEvent, FeedSource, FeedStats, GapPolicy};

/// Length of the `MoldUDP64` packet header in bytes.
pub const MOLD_UDP64_HEADER_LEN: usize = 20;

/// The `0xFFFF` message count that marks an end-of-session packet.
pub const END_OF_SESSION_MESSAGE_COUNT: u16 = 0xFFFF;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MoldUDP64PacketHeader {
    pub session: [u8; 10],
    pub sequence_number: u64,
    pub message_count: u16,
}

/// Splits a `MoldUDP64` packet into its header and message body.
///
/// Returns `None` if fewer than [`MOLD_UDP64_HEADER_LEN`] bytes are present.
#[must_use]
pub fn parse_header(body: &[u8]) -> Option<(MoldUDP64PacketHeader, &[u8])> {
    let (message_header, message_body) = body.split_at_checked(MOLD_UDP64_HEADER_LEN)?;
    Some((
        MoldUDP64PacketHeader {
            session: message_header[..10].try_into().ok()?,
            sequence_number: u64::from_be_bytes(message_header[10..18].try_into().ok()?),
            message_count: u16::from_be_bytes(message_header[18..20].try_into().ok()?),
        },
        message_body,
    ))
}

/// How a call to [`parse_messages`] finished.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct MessageOutcome {
    /// How many message blocks were handed to the callback.
    pub delivered: u16,
    /// Whether the body ended before all declared messages were read. A
    /// truncated packet means the transport lost bytes, and the messages that
    /// *were* delivered are only trustworthy up to this point.
    pub truncated: bool,
}

/// Iterates the `message_count` length-prefixed message blocks in a packet body.
///
/// Stops early if the body is shorter than the declared messages, reporting
/// that in the returned [`MessageOutcome`] rather than failing silently.
pub fn parse_messages(
    message_body: &[u8],
    message_count: u16,
    mut handle: impl FnMut(&[u8]),
) -> MessageOutcome {
    let mut buf = message_body;
    let mut delivered = 0;

    for _ in 0..message_count {
        let Some((message_length_bytes, rest)) = buf.split_at_checked(2) else {
            return MessageOutcome {
                delivered,
                truncated: true,
            };
        };
        let message_length =
            u16::from_be_bytes([message_length_bytes[0], message_length_bytes[1]]) as usize;
        let Some((message_data, rest)) = rest.split_at_checked(message_length) else {
            return MessageOutcome {
                delivered,
                truncated: true,
            };
        };
        handle(message_data);
        delivered += 1;
        buf = rest;
    }

    MessageOutcome {
        delivered,
        truncated: false,
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PacketStatus {
    NewSession { previous: Option<[u8; 10]> },
    InOrder,
    Gap { start: u64, count: u64 },
    StaleOrDuplicate,
    EndOfSession,
}

/// Tracks `MoldUDP64` session identity and expected sequence number.
#[derive(Debug, Clone, Copy)]
pub struct Session {
    id: Option<[u8; 10]>,
    expected_sequence_number: u64,
    policy: GapPolicy,
    desynced: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            id: None,
            expected_sequence_number: 0,
            policy: GapPolicy::Halt,
            desynced: false,
        }
    }

    #[must_use]
    pub const fn with_policy(policy: GapPolicy) -> Self {
        Self {
            id: None,
            expected_sequence_number: 0,
            policy,
            desynced: false,
        }
    }

    #[must_use]
    pub const fn expected_sequence_number(&self) -> u64 {
        self.expected_sequence_number
    }

    #[must_use]
    pub const fn session_id(&self) -> Option<[u8; 10]> {
        self.id
    }

    #[must_use]
    pub const fn policy(&self) -> GapPolicy {
        self.policy
    }

    /// True when a gap is outstanding: the next expected message was never
    /// seen, so any later message must not be applied to state.
    #[must_use]
    pub const fn is_desynced(&self) -> bool {
        self.desynced
    }

    pub fn on_packet(&mut self, packet_header: &MoldUDP64PacketHeader) -> PacketStatus {
        let sequence_number = packet_header.sequence_number;
        let is_end_of_session = packet_header.message_count == END_OF_SESSION_MESSAGE_COUNT;
        let advance_by = if is_end_of_session {
            0
        } else {
            u64::from(packet_header.message_count)
        };
        // Saturating because a corrupt header can put the sequence number
        // arbitrarily close to u64::MAX; wrapping there would silently unsync
        // the session rather than merely overflowing.
        let end = sequence_number.saturating_add(advance_by);

        if self.id != Some(packet_header.session) {
            let previous = self.id.replace(packet_header.session);
            self.expected_sequence_number = end;
            self.desynced = false;
            return PacketStatus::NewSession { previous };
        }

        if is_end_of_session {
            return PacketStatus::EndOfSession;
        }

        if self.desynced {
            // Waiting for the missing range. Only a packet that lands exactly
            // on the expected sequence number can re-establish order.
            if sequence_number == self.expected_sequence_number {
                self.desynced = false;
                self.expected_sequence_number = end;
                return PacketStatus::InOrder;
            }
            return if sequence_number < self.expected_sequence_number {
                PacketStatus::StaleOrDuplicate
            } else {
                // Still open, and possibly wider than before.
                PacketStatus::Gap {
                    start: self.expected_sequence_number,
                    count: sequence_number - self.expected_sequence_number,
                }
            };
        }

        match sequence_number.cmp(&self.expected_sequence_number) {
            std::cmp::Ordering::Equal => {
                self.expected_sequence_number = end;
                PacketStatus::InOrder
            }
            std::cmp::Ordering::Greater => {
                let start = self.expected_sequence_number;
                let count = sequence_number - start;
                match self.policy {
                    GapPolicy::Halt => self.desynced = true,
                    GapPolicy::Skip => self.expected_sequence_number = end,
                }
                PacketStatus::Gap { start, count }
            }
            std::cmp::Ordering::Less => {
                // Retransmission requests are assumed to cover exactly the
                // requested range, so even a partial overlap counts as fully
                // stale rather than being replayed.
                PacketStatus::StaleOrDuplicate
            }
        }
    }
}

/// Builds a `MoldUDP64` packet from the given messages.
///
/// # Panics
///
/// Panics if `messages.len()` exceeds `u16::MAX`, if any individual message
/// exceeds `u16::MAX` bytes, or if `messages.len()` is exactly `0xFFFF`
/// (reserved for End of Session).
#[must_use]
pub fn build_packet(session: [u8; 10], sequence_number: u64, messages: &[&[u8]]) -> Vec<u8> {
    let message_count = u16::try_from(messages.len())
        .expect("message count exceeds u16::MAX (MoldUDP64 message_count is a u16)");
    assert_ne!(
        message_count, END_OF_SESSION_MESSAGE_COUNT,
        "message_count 0xFFFF is reserved for End of Session"
    );

    let payload_len: usize = messages.iter().map(|m| 2 + m.len()).sum();
    let mut buf = Vec::with_capacity(MOLD_UDP64_HEADER_LEN + payload_len);

    buf.extend_from_slice(&session);
    buf.extend_from_slice(&sequence_number.to_be_bytes());
    buf.extend_from_slice(&message_count.to_be_bytes());

    for message in messages {
        let message_length = u16::try_from(message.len())
            .expect("message length exceeds u16::MAX (MoldUDP64 message length is a u16)");
        buf.extend_from_slice(&message_length.to_be_bytes());
        buf.extend_from_slice(message);
    }

    buf
}

#[must_use]
pub fn build_end_of_session_packet(session: [u8; 10], sequence_number: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(MOLD_UDP64_HEADER_LEN);
    buf.extend_from_slice(&session);
    buf.extend_from_slice(&sequence_number.to_be_bytes());
    buf.extend_from_slice(&END_OF_SESSION_MESSAGE_COUNT.to_be_bytes());
    buf
}

/// Drains a feed, dispatching every event and message.
///
/// The event callback returns [`ControlFlow`] so a consumer can stop the feed
/// — for example on a gap it cannot recover from, or on
/// [`FeedEvent::EndOfSession`].
///
/// # Errors
///
/// Returns an error if the underlying source fails while producing the next
/// item.
pub fn run_feed(
    mut source: impl FeedSource,
    mut on_event: impl FnMut(FeedEvent) -> ControlFlow<()>,
    mut on_message: impl FnMut(&[u8]),
) -> std::io::Result<FeedStats> {
    while let Some(feed) = source.next()? {
        if feed.event.is_some_and(|event| on_event(event).is_break()) {
            break;
        }
        if let Some(message) = feed.message {
            on_message(message);
        }
    }
    Ok(source.stats())
}

// <Generated by Claude.ai>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::itch::{ParsedMessage, SystemEventMessage, parse_message};

    fn session_id(byte: u8) -> [u8; 10] {
        [byte; 10]
    }

    fn header_bytes(session: [u8; 10], sequence_number: u64, message_count: u16) -> Vec<u8> {
        let mut buf = Vec::with_capacity(MOLD_UDP64_HEADER_LEN);
        buf.extend_from_slice(&session);
        buf.extend_from_slice(&sequence_number.to_be_bytes());
        buf.extend_from_slice(&message_count.to_be_bytes());
        buf
    }

    fn message_block(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(2 + payload.len());
        buf.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    fn header(session: u8, sequence_number: u64, message_count: u16) -> MoldUDP64PacketHeader {
        MoldUDP64PacketHeader {
            session: session_id(session),
            sequence_number,
            message_count,
        }
    }

    // ---- parse_header ----

    #[test]
    fn parse_header_valid_with_trailing_body() {
        let mut packet = header_bytes(session_id(1), 42, 3);
        packet.extend_from_slice(b"trailing");

        let (parsed, rest) = parse_header(&packet).expect("should parse");
        assert_eq!(parsed, header(1, 42, 3));
        assert_eq!(rest, b"trailing");
    }

    #[test]
    fn parse_header_exact_20_bytes_leaves_empty_body() {
        let packet = header_bytes(session_id(1), 1, 0);
        let (_, rest) = parse_header(&packet).expect("should parse");
        assert!(rest.is_empty());
    }

    #[test]
    fn parse_header_truncated_returns_none() {
        assert!(parse_header(&[0u8; 19]).is_none());
    }

    #[test]
    fn parse_header_empty_returns_none() {
        assert!(parse_header(&[]).is_none());
    }

    // ---- parse_messages ----

    #[test]
    fn parse_messages_collects_all_blocks_in_order() {
        let mut body = Vec::new();
        body.extend(message_block(b"AAPL"));
        body.extend(message_block(b"MSFT"));
        body.extend(message_block(b""));

        let mut seen = Vec::new();
        let outcome = parse_messages(&body, 3, |msg| seen.push(msg.to_vec()));

        assert_eq!(seen, vec![b"AAPL".to_vec(), b"MSFT".to_vec(), b"".to_vec()]);
        assert_eq!(
            outcome,
            MessageOutcome {
                delivered: 3,
                truncated: false
            }
        );
    }

    #[test]
    fn parse_messages_zero_count_calls_handle_zero_times() {
        let body = message_block(b"unused");
        let mut calls = 0;
        let outcome = parse_messages(&body, 0, |_| calls += 1);
        assert_eq!(calls, 0);
        assert!(!outcome.truncated);
    }

    #[test]
    fn parse_messages_reports_truncation_instead_of_failing_silently() {
        let body = vec![0u8]; // cannot even hold a length prefix
        let outcome = parse_messages(&body, 5, |_| {});
        assert_eq!(outcome.delivered, 0);
        assert!(outcome.truncated);
    }

    #[test]
    fn parse_messages_reports_a_declared_length_beyond_the_buffer() {
        let mut body = Vec::new();
        body.extend_from_slice(&10u16.to_be_bytes());
        body.extend_from_slice(b"only3");

        let outcome = parse_messages(&body, 1, |_| {});
        assert_eq!(outcome.delivered, 0);
        assert!(outcome.truncated);
    }

    #[test]
    fn parse_messages_reports_partial_delivery_before_truncation() {
        let mut body = message_block(b"good");
        body.push(0); // half a length prefix

        let mut seen = Vec::new();
        let outcome = parse_messages(&body, 2, |msg| seen.push(msg.to_vec()));

        assert_eq!(seen, vec![b"good".to_vec()]);
        assert_eq!(outcome.delivered, 1);
        assert!(outcome.truncated);
    }

    // ---- Session ----

    #[test]
    fn first_packet_reports_new_session_with_no_previous() {
        let mut session = Session::new();
        let status = session.on_packet(&header(1, 100, 5));

        assert_eq!(status, PacketStatus::NewSession { previous: None });
        assert_eq!(session.expected_sequence_number(), 105);
        assert!(!session.is_desynced());
    }

    #[test]
    fn in_order_packet_advances_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4));

        assert_eq!(session.on_packet(&header(1, 5, 3)), PacketStatus::InOrder);
        assert_eq!(session.expected_sequence_number(), 8);
    }

    #[test]
    fn heartbeat_with_matching_sequence_is_in_order_and_does_not_advance() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4));

        assert_eq!(session.on_packet(&header(1, 5, 0)), PacketStatus::InOrder);
        assert_eq!(session.expected_sequence_number(), 5);
    }

    #[test]
    fn gap_reports_start_and_count() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4)); // expects 5

        assert_eq!(
            session.on_packet(&header(1, 10, 2)),
            PacketStatus::Gap { start: 5, count: 5 }
        );
    }

    #[test]
    fn halt_policy_does_not_advance_past_a_gap() {
        let mut session = Session::with_policy(GapPolicy::Halt);
        session.on_packet(&header(1, 1, 4)); // expects 5

        session.on_packet(&header(1, 10, 2));
        assert_eq!(
            session.expected_sequence_number(),
            5,
            "Halt must keep waiting for the missing range"
        );
        assert!(session.is_desynced());
    }

    #[test]
    fn skip_policy_advances_past_a_gap() {
        let mut session = Session::with_policy(GapPolicy::Skip);
        session.on_packet(&header(1, 1, 4)); // expects 5

        session.on_packet(&header(1, 10, 2));
        assert_eq!(session.expected_sequence_number(), 12);
        assert!(!session.is_desynced());
    }

    #[test]
    fn halt_policy_resyncs_when_the_expected_packet_arrives() {
        let mut session = Session::with_policy(GapPolicy::Halt);
        session.on_packet(&header(1, 1, 4)); // expects 5
        session.on_packet(&header(1, 10, 2)); // gap 5..10

        assert_eq!(session.on_packet(&header(1, 5, 2)), PacketStatus::InOrder);
        assert!(!session.is_desynced());
        assert_eq!(session.expected_sequence_number(), 7);
    }

    #[test]
    fn halt_policy_keeps_reporting_a_widening_gap() {
        let mut session = Session::with_policy(GapPolicy::Halt);
        session.on_packet(&header(1, 1, 4)); // expects 5
        session.on_packet(&header(1, 10, 2));

        assert_eq!(
            session.on_packet(&header(1, 20, 1)),
            PacketStatus::Gap {
                start: 5,
                count: 15
            }
        );
        assert!(session.is_desynced());
    }

    #[test]
    fn halt_policy_treats_a_packet_below_expected_as_stale() {
        let mut session = Session::with_policy(GapPolicy::Halt);
        session.on_packet(&header(1, 1, 4)); // expects 5
        session.on_packet(&header(1, 10, 2)); // desynced

        assert_eq!(
            session.on_packet(&header(1, 3, 1)),
            PacketStatus::StaleOrDuplicate
        );
    }

    #[test]
    fn stale_or_duplicate_does_not_move_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4)); // expects 5

        assert_eq!(
            session.on_packet(&header(1, 1, 4)),
            PacketStatus::StaleOrDuplicate
        );
        assert_eq!(session.expected_sequence_number(), 5);
    }

    #[test]
    fn end_of_session_does_not_move_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4));

        assert_eq!(
            session.on_packet(&header(1, 5, END_OF_SESSION_MESSAGE_COUNT)),
            PacketStatus::EndOfSession
        );
        assert_eq!(session.expected_sequence_number(), 5);
    }

    #[test]
    fn session_change_mid_stream_reports_previous_and_reseeds_expected() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4));

        assert_eq!(
            session.on_packet(&header(2, 50, 2)),
            PacketStatus::NewSession {
                previous: Some(session_id(1))
            }
        );
        assert_eq!(session.expected_sequence_number(), 52);
    }

    #[test]
    fn a_new_session_clears_a_pending_gap() {
        let mut session = Session::with_policy(GapPolicy::Halt);
        session.on_packet(&header(1, 1, 4));
        session.on_packet(&header(1, 10, 2));
        assert!(session.is_desynced());

        session.on_packet(&header(2, 1, 1));
        assert!(!session.is_desynced());
    }

    #[test]
    fn first_packet_being_end_of_session_does_not_corrupt_expected_sequence_number() {
        let mut session = Session::new();
        let status = session.on_packet(&header(1, 100, END_OF_SESSION_MESSAGE_COUNT));

        assert_eq!(status, PacketStatus::NewSession { previous: None });
        assert_eq!(session.expected_sequence_number(), 100);
    }

    #[test]
    fn end_of_session_takes_priority_over_gap_and_stale_sequence_values() {
        let mut ahead = Session::new();
        ahead.on_packet(&header(1, 1, 4));
        assert_eq!(
            ahead.on_packet(&header(1, 50, END_OF_SESSION_MESSAGE_COUNT)),
            PacketStatus::EndOfSession
        );

        let mut behind = Session::new();
        behind.on_packet(&header(1, 1, 4));
        assert_eq!(
            behind.on_packet(&header(1, 2, END_OF_SESSION_MESSAGE_COUNT)),
            PacketStatus::EndOfSession
        );
    }

    #[test]
    fn duplicate_heartbeat_is_stale_and_does_not_advance() {
        let mut session = Session::new();
        session.on_packet(&header(1, 1, 4)); // expects 5

        assert_eq!(
            session.on_packet(&header(1, 3, 0)),
            PacketStatus::StaleOrDuplicate
        );
        assert_eq!(session.expected_sequence_number(), 5);
    }

    #[test]
    fn a_corrupt_sequence_near_u64_max_saturates_instead_of_wrapping() {
        let mut session = Session::new();
        session.on_packet(&header(1, u64::MAX - 1, 1));

        assert_eq!(
            session.expected_sequence_number(),
            u64::MAX,
            "saturating_add keeps us at the ceiling rather than wrapping to 0"
        );
    }

    // ---- build_packet ----

    #[test]
    fn build_packet_round_trips_through_parse_header_and_parse_messages() {
        let packet = build_packet(session_id(1), 7, &[b"ONE", b"TWO"]);
        let (parsed, body) = parse_header(&packet).expect("should parse");

        assert_eq!(parsed, header(1, 7, 2));

        let mut seen = Vec::new();
        let outcome = parse_messages(body, parsed.message_count, |m| seen.push(m.to_vec()));
        assert_eq!(seen, vec![b"ONE".to_vec(), b"TWO".to_vec()]);
        assert!(!outcome.truncated);
    }

    #[test]
    fn end_of_session_packet_uses_the_reserved_count() {
        let packet = build_end_of_session_packet(session_id(1), 99);
        let (parsed, body) = parse_header(&packet).expect("should parse");

        assert_eq!(parsed.message_count, END_OF_SESSION_MESSAGE_COUNT);
        assert!(body.is_empty());
    }

    // ---- End-to-end ----

    #[test]
    fn a_pipeline_message_reaches_parse_message_intact() {
        // Hand-build a minimal, real System Event message.
        let mut system_event = Vec::with_capacity(12);
        system_event.push(b'S');
        system_event.extend_from_slice(&1u16.to_be_bytes());
        system_event.extend_from_slice(&1u16.to_be_bytes());
        system_event.extend_from_slice(&34_200_000_000_000u64.to_be_bytes()[2..8]);
        system_event.push(b'O');

        let packet = build_packet(session_id(1), 1, &[system_event.as_slice()]);
        let (parsed_header, body) = parse_header(&packet).expect("should parse header");

        let mut parsed_messages = Vec::new();
        parse_messages(body, parsed_header.message_count, |message| {
            parsed_messages.push(parse_message(message));
        });

        assert_eq!(parsed_messages.len(), 1);
        assert_eq!(
            parsed_messages[0],
            Ok(ParsedMessage::SystemEvent(SystemEventMessage {
                stock_locate: 1,
                tracking_number: 1,
                timestamp: 34_200_000_000_000,
                event_code: itch::EventCode::StartOfMessages,
            }))
        );
    }
}
// </Generated by Claude.ai>

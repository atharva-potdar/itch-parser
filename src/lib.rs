use crate::source::PacketSource;

pub mod itch;
pub mod source;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MoldUDP64PacketHeader {
    pub session: [u8; 10],
    pub sequence_number: u64,
    pub message_count: u16,
}

#[must_use]
pub fn parse_header(body: &[u8]) -> Option<(MoldUDP64PacketHeader, &[u8])> {
    let (message_header, message_body) = body.split_at_checked(20)?;
    Some((
        MoldUDP64PacketHeader {
            session: message_header[..10].try_into().ok()?,
            sequence_number: u64::from_be_bytes(message_header[10..18].try_into().ok()?),
            message_count: u16::from_be_bytes(message_header[18..20].try_into().ok()?),
        },
        message_body,
    ))
}

pub fn parse_messages(message_body: &[u8], message_count: u16, mut handle: impl FnMut(&[u8])) {
    let mut buf = message_body;
    for _ in 0..message_count {
        let Some((message_length_bytes, rest)) = buf.split_at_checked(2) else {
            return;
        };
        let message_length =
            u16::from_be_bytes([message_length_bytes[0], message_length_bytes[1]]) as usize;
        let Some((message_data, rest)) = rest.split_at_checked(message_length) else {
            return;
        };
        handle(message_data);
        buf = rest;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PacketStatus {
    NewSession { previous: Option<[u8; 10]> },
    InOrder,
    Gap { start: u64, count: u64 },
    StaleOrDuplicate,
    EndOfSession,
}

pub struct Session {
    session_id: Option<[u8; 10]>,
    expected_sequence_number: u64,
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
            session_id: None,
            expected_sequence_number: 0,
        }
    }

    #[must_use]
    pub const fn expected_sequence_number(&self) -> u64 {
        self.expected_sequence_number
    }

    #[must_use]
    pub const fn session_id(&self) -> Option<[u8; 10]> {
        self.session_id
    }

    pub fn on_packet(&mut self, packet_header: &MoldUDP64PacketHeader) -> PacketStatus {
        let sequence_number = packet_header.sequence_number;
        let is_end_of_session = packet_header.message_count == 0xFFFF;
        let advance_by = if is_end_of_session {
            0
        } else {
            u64::from(packet_header.message_count)
        };
        let end = sequence_number + advance_by;

        if self.session_id != Some(packet_header.session) {
            let previous = self.session_id.replace(packet_header.session);
            self.expected_sequence_number = end;
            return PacketStatus::NewSession { previous };
        }

        if is_end_of_session {
            return PacketStatus::EndOfSession;
        }

        match sequence_number.cmp(&self.expected_sequence_number) {
            std::cmp::Ordering::Equal => {
                self.expected_sequence_number = end;
                PacketStatus::InOrder
            }
            std::cmp::Ordering::Greater => {
                let old_expected_sequence_number = self.expected_sequence_number;
                let gap = sequence_number - self.expected_sequence_number;
                self.expected_sequence_number = end;
                PacketStatus::Gap {
                    start: old_expected_sequence_number,
                    count: gap,
                }
            }
            std::cmp::Ordering::Less => {
                // Assume that retransmission requests cover exactly the requested
                // range, so that even if there is partial overlap, it counts as
                // fully stale
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
        message_count, 0xFFFF,
        "message_count 0xFFFF is reserved for End of Session"
    );

    let payload_len: usize = messages.iter().map(|m| 2 + m.len()).sum();
    let mut buf = Vec::with_capacity(20 + payload_len);

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
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&session);
    buf.extend_from_slice(&sequence_number.to_be_bytes());
    buf.extend_from_slice(&0xFFFFu16.to_be_bytes());
    buf
}

/// # Errors
///
/// Returns an error if `source.next_packet()` fails while reading the
/// stream.
pub fn run_pipeline(
    mut source: impl PacketSource,
    session: &mut Session,
    mut on_status: impl FnMut(&MoldUDP64PacketHeader, &PacketStatus),
    mut on_message: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    let mut buf = Vec::new();

    while let Some(received) = source.next_packet(&mut buf)? {
        let packet = &buf[..received];
        let Some((header, body)) = parse_header(packet) else {
            continue; // malformed packet; skip rather than abort the whole stream
        };

        let status = session.on_packet(&header);
        on_status(&header, &status);

        let should_deliver = match status {
            PacketStatus::InOrder | PacketStatus::Gap { .. } => true,
            PacketStatus::NewSession { .. } => header.message_count != 0xFFFF,
            PacketStatus::StaleOrDuplicate | PacketStatus::EndOfSession => false,
        };

        if should_deliver {
            parse_messages(body, header.message_count, &mut on_message);
        }
    }
    Ok(())
}

// <Generated by Claude.ai>
#[cfg(test)]
mod tests {
    use super::*;

    fn session_id(byte: u8) -> [u8; 10] {
        [byte; 10]
    }

    fn header_bytes(session: [u8; 10], sequence_number: u64, message_count: u16) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20);
        buf.extend_from_slice(&session);
        buf.extend_from_slice(&sequence_number.to_be_bytes());
        buf.extend_from_slice(&message_count.to_be_bytes());
        buf
    }

    fn message_block(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(2 + payload.len());
        buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    // ---- parse_header ----

    #[test]
    fn parse_header_valid_with_trailing_body() {
        let mut packet = header_bytes(session_id(1), 42, 3);
        packet.extend_from_slice(b"trailing");

        let (header, rest) = parse_header(&packet).expect("should parse");
        assert_eq!(
            header,
            MoldUDP64PacketHeader {
                session: session_id(1),
                sequence_number: 42,
                message_count: 3,
            }
        );
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
        let packet = vec![0u8; 19];
        assert!(parse_header(&packet).is_none());
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
        parse_messages(&body, 3, |msg| seen.push(msg.to_vec()));

        assert_eq!(seen, vec![b"AAPL".to_vec(), b"MSFT".to_vec(), b"".to_vec()]);
    }

    #[test]
    fn parse_messages_zero_count_calls_handle_zero_times() {
        let body = message_block(b"unused");
        let mut calls = 0;
        parse_messages(&body, 0, |_| calls += 1);
        assert_eq!(calls, 0);
    }

    #[test]
    fn parse_messages_stops_silently_when_length_prefix_truncated() {
        let body = vec![0u8]; // one byte, can't even read a u16 length
        let mut calls = 0;
        parse_messages(&body, 5, |_| calls += 1);
        assert_eq!(calls, 0);
    }

    #[test]
    fn parse_messages_stops_silently_when_declared_length_exceeds_buffer() {
        let mut body = Vec::new();
        body.extend_from_slice(&10u16.to_be_bytes()); // claims 10 bytes follow
        body.extend_from_slice(b"only3"); // but only 5 are present

        let mut calls = 0;
        parse_messages(&body, 1, |_| calls += 1);
        assert_eq!(calls, 0);
    }

    #[test]
    fn parse_messages_stops_after_partial_success_on_truncation() {
        // one good message, then a second block whose length prefix is cut off
        let mut body = message_block(b"good");
        body.push(0); // half a length prefix

        let mut seen = Vec::new();
        parse_messages(&body, 2, |msg| seen.push(msg.to_vec()));

        assert_eq!(seen, vec![b"good".to_vec()]);
    }

    // ---- Session ----

    #[test]
    fn first_packet_reports_new_session_with_no_previous() {
        let mut session = Session::new();
        let header = MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 100,
            message_count: 5,
        };

        let status = session.on_packet(&header);

        assert_eq!(status, PacketStatus::NewSession { previous: None });
        assert_eq!(session.expected_sequence_number, 105);
    }

    #[test]
    fn in_order_packet_advances_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        });

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 5,
            message_count: 3,
        });

        assert_eq!(status, PacketStatus::InOrder);
        assert_eq!(session.expected_sequence_number, 8);
    }

    #[test]
    fn heartbeat_with_matching_sequence_is_in_order_and_does_not_advance() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        });

        // heartbeat: message_count 0, sequence_number == next expected
        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 5,
            message_count: 0,
        });

        assert_eq!(status, PacketStatus::InOrder);
        assert_eq!(session.expected_sequence_number, 5);
    }

    #[test]
    fn gap_reports_start_and_count_and_advances_past_it() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5 next

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 10,
            message_count: 2,
        });

        assert_eq!(status, PacketStatus::Gap { start: 5, count: 5 });
        assert_eq!(session.expected_sequence_number, 12);
    }

    #[test]
    fn stale_or_duplicate_does_not_move_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5 next

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        });

        assert_eq!(status, PacketStatus::StaleOrDuplicate);
        assert_eq!(session.expected_sequence_number, 5);
    }

    #[test]
    fn end_of_session_does_not_move_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5 next

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 5,
            message_count: 0xFFFF,
        });

        assert_eq!(status, PacketStatus::EndOfSession);
        assert_eq!(session.expected_sequence_number, 5);
    }

    #[test]
    fn session_change_mid_stream_reports_previous_and_reseeds_expected() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        });

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(2),
            sequence_number: 50,
            message_count: 2,
        });

        assert_eq!(
            status,
            PacketStatus::NewSession {
                previous: Some(session_id(1))
            }
        );
        assert_eq!(session.expected_sequence_number, 52);
    }

    #[test]
    fn first_packet_being_end_of_session_does_not_corrupt_expected_sequence_number() {
        let mut session = Session::new();
        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 100,
            message_count: 0xFFFF,
        });

        assert_eq!(status, PacketStatus::NewSession { previous: None });
        assert_eq!(session.expected_sequence_number, 100); // not 100 + 65535
    }

    #[test]
    fn new_sessions_first_packet_being_end_of_session_does_not_corrupt_expected_sequence_number() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        });

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(2),
            sequence_number: 200,
            message_count: 0xFFFF,
        });

        assert_eq!(
            status,
            PacketStatus::NewSession {
                previous: Some(session_id(1))
            }
        );
        assert_eq!(session.expected_sequence_number, 200);
    }

    #[test]
    fn end_of_session_takes_priority_over_gap_and_stale_sequence_values() {
        let mut ahead = Session::new();
        ahead.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5
        assert_eq!(
            ahead.on_packet(&MoldUDP64PacketHeader {
                session: session_id(1),
                sequence_number: 50,
                message_count: 0xFFFF
            }),
            PacketStatus::EndOfSession
        );

        let mut behind = Session::new();
        behind.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5
        assert_eq!(
            behind.on_packet(&MoldUDP64PacketHeader {
                session: session_id(1),
                sequence_number: 2,
                message_count: 0xFFFF
            }),
            PacketStatus::EndOfSession
        );
    }

    #[test]
    fn heartbeat_ahead_of_expected_reports_gap() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 9,
            message_count: 0,
        });

        assert_eq!(status, PacketStatus::Gap { start: 5, count: 4 });
        assert_eq!(session.expected_sequence_number, 9);
    }

    #[test]
    fn duplicate_heartbeat_is_stale_and_does_not_advance() {
        let mut session = Session::new();
        session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 1,
            message_count: 4,
        }); // expects 5

        let status = session.on_packet(&MoldUDP64PacketHeader {
            session: session_id(1),
            sequence_number: 3,
            message_count: 0,
        });

        assert_eq!(status, PacketStatus::StaleOrDuplicate);
        assert_eq!(session.expected_sequence_number, 5);
    }

    #[test]
    fn run_pipeline_delivers_message_that_itch_parse_message_can_parse() {
        use crate::itch::{parse_message, ParsedMessage, SystemEventMessage};

        // Hand-build a minimal, real System Event message
        let mut system_event = Vec::with_capacity(12);
        system_event.push(b'S');
        system_event.extend_from_slice(&1u16.to_be_bytes()); // stock_locate
        system_event.extend_from_slice(&1u16.to_be_bytes()); // tracking_number
        system_event.extend_from_slice(&34_200_000_000_000u64.to_be_bytes()[2..8]); // timestamp
        system_event.push(b'O'); // event_code

        let packet = build_packet(session_id(1), 1, &[system_event.as_slice()]);

        let (header, body) = parse_header(&packet).expect("should parse MoldUDP64 header");

        let mut parsed_messages = Vec::new();
        parse_messages(body, header.message_count, |message_bytes| {
            parsed_messages.push(parse_message(message_bytes));
        });

        assert_eq!(parsed_messages.len(), 1);
        assert_eq!(
            parsed_messages[0],
            Some(ParsedMessage::SystemEvent(SystemEventMessage {
                stock_locate: 1,
                tracking_number: 1,
                timestamp: 34_200_000_000_000,
                event_code: b'O',
            }))
        );
    }
}
// </Generated by Claude.ai>

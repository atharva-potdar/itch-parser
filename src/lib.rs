struct MoldUDP64PacketHeader {
    session: [u8; 10],
    sequence_number: u64,
    message_count: u16
}


fn parse_header(body: &[u8]) -> Option<(MoldUDP64PacketHeader, &[u8])> {
    let (message_header, message_body) = body.split_at_checked(20)?;
    Some((
        MoldUDP64PacketHeader {
            session: message_header[..10].try_into().ok()?,
            sequence_number: u64::from_be_bytes(message_header[10..18].try_into().ok()?),
            message_count: u16::from_be_bytes(message_header[18..20].try_into().ok()?)
        },
        message_body
    ))
}


fn parse_messages(message_body: &[u8], message_count: u16, mut handle: impl FnMut(&[u8])) {
    let mut buf = message_body;
    for _ in 0..message_count {
        let Some((message_length_bytes, rest)) = buf.split_at_checked(2) else { return };
        let message_length = u16::from_be_bytes(message_length_bytes.try_into().unwrap()) as usize;
        let Some((message_data, rest)) = rest.split_at_checked(message_length) else { return };
        handle(message_data);
        buf = rest;
    }
}

enum PacketStatus {
    InOrder,
    Gap(u64),
    StaleOrDuplicate,
}

struct Session {
    expected_sequence_number: u64,
}

impl Session {
    fn new(start: u64) -> Self { Self { expected_sequence_number: start } }

    fn on_packet(&mut self, packet_header: &MoldUDP64PacketHeader) -> PacketStatus {
        let sequence_number = packet_header.sequence_number;
        let end = sequence_number + packet_header.message_count as u64;
        if sequence_number == self.expected_sequence_number {
            self.expected_sequence_number = end;
            PacketStatus::InOrder
        } else if sequence_number > self.expected_sequence_number {
            let gap = sequence_number - self.expected_sequence_number;
            self.expected_sequence_number = end;
            PacketStatus::Gap(gap)
        } else {
            PacketStatus::StaleOrDuplicate
        }
    }
}
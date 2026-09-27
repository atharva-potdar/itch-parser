struct MoldUDP64PacketHeader {
    session: [u8; 10],
    sequence_number: u64,
    message_count: u16
}
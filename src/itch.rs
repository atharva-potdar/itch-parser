#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MessageType {
    SystemEvent,        // 'S'
}

impl MessageType {
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'S' => Some(Self::SystemEvent),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct SystemEventMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64, // ns since midnight, 6 byte length
    pub event_code: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParsedMessage {
    SystemEvent(SystemEventMessage),
}

#[must_use]
pub fn parse_message(message: &[u8]) -> Option<ParsedMessage> {
    let type_byte = *message.first()?;
    match MessageType::from_byte(type_byte)? {
        MessageType::SystemEvent => parse_system_event(message).map(ParsedMessage::SystemEvent),
    }
}

/// Reads a 6-byte big-endian value (ITCH's on-wire timestamp width) and
/// widens it into a `u64`. `bytes` must be exactly 6 bytes long.
fn read_u48_be(bytes: &[u8]) -> u64 {
    let mut widened = [0u8; 8];
    widened[2..8].copy_from_slice(bytes);
    u64::from_be_bytes(widened)
}

pub const SYSTEM_EVENT_MESSAGE_LEN: usize = 12;

#[must_use]
pub fn parse_system_event(message: &[u8]) -> Option<SystemEventMessage> {
    if message.len() != SYSTEM_EVENT_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'S' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);

    let timestamp = read_u48_be(&message[5..11]);

    let event_code = message[11];

    Some(SystemEventMessage {
        stock_locate,
        tracking_number,
        timestamp,
        event_code,
    })
}
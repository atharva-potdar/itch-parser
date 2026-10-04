#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MessageType {
    SystemEvent,               // 'S'
    StockDirectory,            // 'R'
    StockTradingAction,        // 'H'
    RegSHORestriction,         // 'Y'
    MarketParticipantPosition, // 'L'
    MWCBDeclineLevel,          // 'V'
    MWCBStatus,                // 'W'
    IPOQuotingPeriodUpdate,    // 'K'
    LULDAuctionCollar,         // 'J'
    OperationalHalt,           // 'h'
    AddOrder,                  // 'A'
    AddOrderMPIDAttribution,   // 'F'
}

impl MessageType {
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'S' => Some(Self::SystemEvent),
            b'R' => Some(Self::StockDirectory),
            b'H' => Some(Self::StockTradingAction),
            b'Y' => Some(Self::RegSHORestriction),
            b'L' => Some(Self::MarketParticipantPosition),
            b'V' => Some(Self::MWCBDeclineLevel),
            b'W' => Some(Self::MWCBStatus),
            b'K' => Some(Self::IPOQuotingPeriodUpdate),
            b'J' => Some(Self::LULDAuctionCollar),
            b'h' => Some(Self::OperationalHalt),
            b'A' => Some(Self::AddOrder),
            b'F' => Some(Self::AddOrderMPIDAttribution),
            _ => None,
        }
    }
}

// all timestamps are u48, nanoseconds since midnight
// all price fields are fixed-point, x10^4 of actual price
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct SystemEventMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub event_code: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct StockDirectoryMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: [u8; 8],
    pub market_category: u8,
    pub financial_status_indicator: u8,
    pub round_lot_size: u32,
    pub round_lots_only: u8,
    pub issue_classification: u8,
    pub issue_subtype: [u8; 2],
    pub authenticity: u8,
    pub short_sale_threshold_indicator: u8,
    pub ipo_flag: u8,
    pub luld_reference_price_tier: u8,
    pub etp_flag: u8,
    pub etp_leverage_factor: u32,
    pub inverse_indicator: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct StockTradingActionMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: [u8; 8],
    pub trading_state: u8,
    pub reserved: u8,
    pub reason: [u8; 4],
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct RegSHORestrictionMessage {
    pub locate_code: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: [u8; 8],
    pub reg_sho_action: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MarketParticipantPositionMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub mpid: [u8; 4],
    pub stock: [u8; 8],
    pub primary_market_maker: u8,
    pub market_maker_mode: u8,
    pub market_participant_state: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MWCBDeclineLevelMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub level1: u64,
    pub level2: u64,
    pub level3: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MWCBStatusMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub breached_level: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct IPOQuotingPeriodUpdateMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: [u8; 8],
    pub ipo_quotation_release_time: u32, // seconds since midnight
    pub ipo_quotation_release_qualifier: u8,
    pub ipo_price: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct LULDAuctionCollarMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: [u8; 8],
    pub auction_collar_reference_price: u32,
    pub upper_auction_collar_price: u32,
    pub lower_auction_collar_price: u32,
    pub auction_collar_extension: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OperationalHaltMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: [u8; 8],
    pub market_code: u8,
    pub operational_halt_action: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct AddOrderMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub buy_sell_indicator: u8,
    pub shares: u32,
    pub stock: [u8; 8],
    pub price: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct AddOrderMPIDAttributionMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub buy_sell_indicator: u8,
    pub shares: u32,
    pub stock: [u8; 8],
    pub price: u32,
    pub attribution: [u8; 4],
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParsedMessage {
    SystemEvent(SystemEventMessage),
    StockDirectory(StockDirectoryMessage),
    StockTradingAction(StockTradingActionMessage),
    RegSHORestriction(RegSHORestrictionMessage),
    MarketParticipantPosition(MarketParticipantPositionMessage),
    MWCBDeclineLevel(MWCBDeclineLevelMessage),
    MWCBStatus(MWCBStatusMessage),
    IPOQuotingPeriodUpdate(IPOQuotingPeriodUpdateMessage),
    LULDAuctionCollar(LULDAuctionCollarMessage),
    OperationalHalt(OperationalHaltMessage),
    AddOrder(AddOrderMessage),
    AddOrderMPIDAttribution(AddOrderMPIDAttributionMessage),
}

#[must_use]
pub fn parse_message(message: &[u8]) -> Option<ParsedMessage> {
    let type_byte = *message.first()?;
    match MessageType::from_byte(type_byte)? {
        MessageType::SystemEvent => parse_system_event(message).map(ParsedMessage::SystemEvent),
        MessageType::StockDirectory => {
            parse_stock_directory(message).map(ParsedMessage::StockDirectory)
        }
        MessageType::StockTradingAction => {
            parse_stock_trading_action(message).map(ParsedMessage::StockTradingAction)
        }
        MessageType::RegSHORestriction => {
            parse_reg_sho_restriction(message).map(ParsedMessage::RegSHORestriction)
        }
        MessageType::MarketParticipantPosition => {
            parse_market_participant_position(message).map(ParsedMessage::MarketParticipantPosition)
        }
        MessageType::MWCBDeclineLevel => {
            parse_mwcb_decline_level(message).map(ParsedMessage::MWCBDeclineLevel)
        }
        MessageType::MWCBStatus => parse_mwcb_status(message).map(ParsedMessage::MWCBStatus),
        MessageType::IPOQuotingPeriodUpdate => {
            parse_ipo_quoting_period_update(message).map(ParsedMessage::IPOQuotingPeriodUpdate)
        }
        MessageType::LULDAuctionCollar => {
            parse_luld_action_collar(message).map(ParsedMessage::LULDAuctionCollar)
        }
        MessageType::OperationalHalt => {
            parse_operational_halt(message).map(ParsedMessage::OperationalHalt)
        }
        MessageType::AddOrder => parse_add_order(message).map(ParsedMessage::AddOrder),
        MessageType::AddOrderMPIDAttribution => {
            parse_add_order_mpid_attribution(message).map(ParsedMessage::AddOrderMPIDAttribution)
        }
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
pub const STOCK_DIRECTORY_MESSAGE_LEN: usize = 39;
pub const STOCK_TRADING_ACTION_MESSAGE_LEN: usize = 25;
pub const REG_SHO_RESTRICTION_MESSAGE_LEN: usize = 20;
pub const MARKET_PARTICIPANT_MESSAGE_LEN: usize = 26;
pub const MWCB_DECLINE_LEVEL_MESSAGE_LEN: usize = 35;
pub const MWCB_STATUS_MESSAGE_LEN: usize = 12;
pub const IPO_QUOTATION_REQUEST_MESSAGE_LEN: usize = 28;
pub const LULD_ACTION_MESSAGE_LEN: usize = 35;
pub const OPERATIONAL_HALT_MESSAGE_LEN: usize = 21;
pub const ADD_ORDER_MESSAGE_LEN: usize = 36;
pub const ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN: usize = 40;

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

#[must_use]
pub fn parse_stock_directory(message: &[u8]) -> Option<StockDirectoryMessage> {
    if message.len() != STOCK_DIRECTORY_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'R' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let stock: [u8; 8] = message[11..19].try_into().ok()?;
    let market_category = message[19];
    let financial_status_indicator = message[20];
    let round_lot_size = u32::from_be_bytes([message[21], message[22], message[23], message[24]]);
    let round_lots_only = message[25];
    let issue_classification = message[26];
    let issue_subtype: [u8; 2] = message[27..29].try_into().ok()?;
    let authenticity = message[29];
    let short_sale_threshold_indicator = message[30];
    let ipo_flag = message[31];
    let luld_reference_price_tier = message[32];
    let etp_flag = message[33];
    let etp_leverage_factor =
        u32::from_be_bytes([message[34], message[35], message[36], message[37]]);
    let inverse_indicator = message[38];

    Some(StockDirectoryMessage {
        stock_locate,
        tracking_number,
        timestamp,
        stock,
        market_category,
        financial_status_indicator,
        round_lot_size,
        round_lots_only,
        issue_classification,
        issue_subtype,
        authenticity,
        short_sale_threshold_indicator,
        ipo_flag,
        luld_reference_price_tier,
        etp_flag,
        etp_leverage_factor,
        inverse_indicator,
    })
}

#[must_use]
pub fn parse_stock_trading_action(message: &[u8]) -> Option<StockTradingActionMessage> {
    if message.len() != STOCK_TRADING_ACTION_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'H' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let stock: [u8; 8] = message[11..19].try_into().ok()?;
    let trading_state: u8 = message[19];
    let reserved = message[20];
    let reason = message[21..25].try_into().ok()?;

    Some(StockTradingActionMessage {
        stock_locate,
        tracking_number,
        timestamp,
        stock,
        trading_state,
        reserved,
        reason,
    })
}

#[must_use]
pub fn parse_reg_sho_restriction(message: &[u8]) -> Option<RegSHORestrictionMessage> {
    if message.len() != REG_SHO_RESTRICTION_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'Y' {
        return None;
    }

    let locate_code = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let stock: [u8; 8] = message[11..19].try_into().ok()?;
    let reg_sho_action = message[19];

    Some(RegSHORestrictionMessage {
        locate_code,
        tracking_number,
        timestamp,
        stock,
        reg_sho_action,
    })
}

#[must_use]
pub fn parse_market_participant_position(
    message: &[u8],
) -> Option<MarketParticipantPositionMessage> {
    if message.len() != MARKET_PARTICIPANT_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'L' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let mpid: [u8; 4] = message[11..15].try_into().ok()?;
    let stock: [u8; 8] = message[15..23].try_into().ok()?;
    let primary_market_maker = message[23];
    let market_maker_mode = message[24];
    let market_participant_state = message[25];

    Some(MarketParticipantPositionMessage {
        stock_locate,
        tracking_number,
        timestamp,
        mpid,
        stock,
        primary_market_maker,
        market_maker_mode,
        market_participant_state,
    })
}

#[must_use]
pub fn parse_mwcb_decline_level(message: &[u8]) -> Option<MWCBDeclineLevelMessage> {
    if message.len() != MWCB_DECLINE_LEVEL_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'V' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let level1 = u64::from_be_bytes([
        message[11],
        message[12],
        message[13],
        message[14],
        message[15],
        message[16],
        message[17],
        message[18],
    ]);
    let level2 = u64::from_be_bytes([
        message[19],
        message[20],
        message[21],
        message[22],
        message[23],
        message[24],
        message[25],
        message[26],
    ]);
    let level3 = u64::from_be_bytes([
        message[27],
        message[28],
        message[29],
        message[30],
        message[31],
        message[32],
        message[33],
        message[34],
    ]);

    Some(MWCBDeclineLevelMessage {
        stock_locate,
        tracking_number,
        timestamp,
        level1,
        level2,
        level3,
    })
}

#[must_use]
pub fn parse_mwcb_status(message: &[u8]) -> Option<MWCBStatusMessage> {
    if message.len() != MWCB_STATUS_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'W' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let breached_level = message[11];

    Some(MWCBStatusMessage {
        stock_locate,
        tracking_number,
        timestamp,
        breached_level,
    })
}

#[must_use]
pub fn parse_ipo_quoting_period_update(message: &[u8]) -> Option<IPOQuotingPeriodUpdateMessage> {
    if message.len() != IPO_QUOTATION_REQUEST_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'K' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let stock: [u8; 8] = message[11..19].try_into().ok()?;
    let ipo_quotation_release_time =
        u32::from_be_bytes([message[19], message[20], message[21], message[22]]);
    let ipo_quotation_release_qualifier = message[23];
    let ipo_price = u32::from_be_bytes([message[24], message[25], message[26], message[27]]);

    Some(IPOQuotingPeriodUpdateMessage {
        stock_locate,
        tracking_number,
        timestamp,
        stock,
        ipo_quotation_release_time,
        ipo_quotation_release_qualifier,
        ipo_price,
    })
}

#[must_use]
pub fn parse_luld_action_collar(message: &[u8]) -> Option<LULDAuctionCollarMessage> {
    if message.len() != LULD_ACTION_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'J' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let stock: [u8; 8] = message[11..19].try_into().ok()?;
    let auction_collar_reference_price =
        u32::from_be_bytes([message[19], message[20], message[21], message[22]]);
    let upper_auction_collar_price =
        u32::from_be_bytes([message[23], message[24], message[25], message[26]]);
    let lower_auction_collar_price =
        u32::from_be_bytes([message[27], message[28], message[29], message[30]]);
    let auction_collar_extension =
        u32::from_be_bytes([message[31], message[32], message[33], message[34]]);

    Some(LULDAuctionCollarMessage {
        stock_locate,
        tracking_number,
        timestamp,
        stock,
        auction_collar_reference_price,
        upper_auction_collar_price,
        lower_auction_collar_price,
        auction_collar_extension,
    })
}

#[must_use]
pub fn parse_operational_halt(message: &[u8]) -> Option<OperationalHaltMessage> {
    if message.len() != OPERATIONAL_HALT_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'h' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let stock: [u8; 8] = message[11..19].try_into().ok()?;
    let market_code = message[19];
    let operational_halt_action = message[20];

    Some(OperationalHaltMessage {
        stock_locate,
        tracking_number,
        timestamp,
        stock,
        market_code,
        operational_halt_action,
    })
}

#[must_use]
pub fn parse_add_order(message: &[u8]) -> Option<AddOrderMessage> {
    if message.len() != ADD_ORDER_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'A' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let order_reference_number = u64::from_be_bytes([
        message[11],
        message[12],
        message[13],
        message[14],
        message[15],
        message[16],
        message[17],
        message[18],
    ]);
    let buy_sell_indicator = message[19];
    let shares = u32::from_be_bytes([message[20], message[21], message[22], message[23]]);
    let stock: [u8; 8] = message[24..32].try_into().ok()?;
    let price = u32::from_be_bytes([message[32], message[33], message[34], message[35]]);

    Some(AddOrderMessage {
        stock_locate,
        tracking_number,
        timestamp,
        order_reference_number,
        buy_sell_indicator,
        shares,
        stock,
        price,
    })
}

#[must_use]
pub fn parse_add_order_mpid_attribution(message: &[u8]) -> Option<AddOrderMPIDAttributionMessage> {
    if message.len() != ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN {
        return None;
    }
    if message[0] != b'F' {
        return None;
    }

    let stock_locate = u16::from_be_bytes([message[1], message[2]]);
    let tracking_number = u16::from_be_bytes([message[3], message[4]]);
    let timestamp = read_u48_be(&message[5..11]);
    let order_reference_number = u64::from_be_bytes([
        message[11],
        message[12],
        message[13],
        message[14],
        message[15],
        message[16],
        message[17],
        message[18],
    ]);
    let buy_sell_indicator = message[19];
    let shares = u32::from_be_bytes([message[20], message[21], message[22], message[23]]);
    let stock: [u8; 8] = message[24..32].try_into().ok()?;
    let price = u32::from_be_bytes([message[32], message[33], message[34], message[35]]);
    let attribution: [u8; 4] = message[36..40].try_into().ok()?;

    Some(AddOrderMPIDAttributionMessage {
        stock_locate,
        tracking_number,
        timestamp,
        order_reference_number,
        buy_sell_indicator,
        shares,
        stock,
        price,
        attribution,
    })
}

// <Generated by Claude.ai>
#[cfg(test)]
mod tests {
    use super::*;

    // ---- SystemEventMessage ----

    fn system_event_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        event_code: u8,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SYSTEM_EVENT_MESSAGE_LEN);
        buf.push(b'S');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]); // low 6 bytes only
        buf.push(event_code);
        assert_eq!(buf.len(), SYSTEM_EVENT_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_system_event_valid() {
        // Distinct, non-zero bytes in every position of the timestamp,
        // specifically so the 6-byte-to-u64 widening is actually exercised.
        let timestamp = 0x0102_0304_0506u64;
        let bytes = system_event_bytes(0x1234, 0x5678, timestamp, b'O');

        let parsed = parse_system_event(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            SystemEventMessage {
                stock_locate: 0x1234,
                tracking_number: 0x5678,
                timestamp,
                event_code: b'O',
            }
        );
    }

    #[test]
    fn parse_system_event_wrong_type_byte_returns_none() {
        let mut bytes = system_event_bytes(1, 2, 0x0102_0304_0506, b'O');
        bytes[0] = b'X';
        assert!(parse_system_event(&bytes).is_none());
    }

    #[test]
    fn parse_system_event_truncated_returns_none() {
        let mut bytes = system_event_bytes(1, 2, 0x0102_0304_0506, b'O');
        bytes.pop();
        assert!(parse_system_event(&bytes).is_none());
    }

    #[test]
    fn parse_message_dispatches_system_event() {
        let bytes = system_event_bytes(1, 2, 0x0102_0304_0506, b'O');
        assert_eq!(
            parse_message(&bytes),
            Some(ParsedMessage::SystemEvent(SystemEventMessage {
                stock_locate: 1,
                tracking_number: 2,
                timestamp: 0x0102_0304_0506,
                event_code: b'O',
            }))
        );
    }

    #[test]
    fn parse_message_unknown_type_byte_returns_none() {
        assert!(parse_message(b"Z\x00\x00\x00\x00").is_none());
    }

    #[test]
    fn parse_message_empty_returns_none() {
        assert!(parse_message(&[]).is_none());
    }

    // ---- StockDirectoryMessage ----

    #[allow(clippy::too_many_arguments)] // test-only builder, mirrors the wire format 1:1
    fn stock_directory_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        stock: [u8; 8],
        market_category: u8,
        financial_status_indicator: u8,
        round_lot_size: u32,
        round_lots_only: u8,
        issue_classification: u8,
        issue_subtype: [u8; 2],
        authenticity: u8,
        short_sale_threshold_indicator: u8,
        ipo_flag: u8,
        luld_reference_price_tier: u8,
        etp_flag: u8,
        etp_leverage_factor: u32,
        inverse_indicator: u8,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(STOCK_DIRECTORY_MESSAGE_LEN);
        buf.push(b'R');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&stock);
        buf.push(market_category);
        buf.push(financial_status_indicator);
        buf.extend_from_slice(&round_lot_size.to_be_bytes());
        buf.push(round_lots_only);
        buf.push(issue_classification);
        buf.extend_from_slice(&issue_subtype);
        buf.push(authenticity);
        buf.push(short_sale_threshold_indicator);
        buf.push(ipo_flag);
        buf.push(luld_reference_price_tier);
        buf.push(etp_flag);
        buf.extend_from_slice(&etp_leverage_factor.to_be_bytes());
        buf.push(inverse_indicator);
        // Sanity check on the builder itself -- if this ever fails, the bug
        // is in the test helper, not in parse_stock_directory.
        assert_eq!(buf.len(), STOCK_DIRECTORY_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_stock_directory_valid() {
        // Every field gets a distinct, recognizable value specifically so
        // that an offset collision (like financial_status_indicator and
        // round_lot_size overlapping, which happened in an earlier draft)
        // or a type mix-up shows up as a mismatch on the exact field that
        // moved, not a vague overall failure.
        let timestamp = 0x0a0b_0c0d_0e0fu64;
        let stock = *b"AAPL    "; // space-padded, as the real feed sends it
        let bytes = stock_directory_bytes(
            0x1111, 0x2222, timestamp, stock, b'Q',   // market_category
            b'N',   // financial_status_indicator
            100,    // round_lot_size
            b'Y',   // round_lots_only
            b'C',   // issue_classification
            *b"XY", // issue_subtype
            b'P',   // authenticity
            b'N',   // short_sale_threshold_indicator
            b'Y',   // ipo_flag
            b'1',   // luld_reference_price_tier
            b'N',   // etp_flag
            9999,   // etp_leverage_factor
            b'N',   // inverse_indicator
        );

        let parsed = parse_stock_directory(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            StockDirectoryMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                stock,
                market_category: b'Q',
                financial_status_indicator: b'N',
                round_lot_size: 100,
                round_lots_only: b'Y',
                issue_classification: b'C',
                issue_subtype: *b"XY",
                authenticity: b'P',
                short_sale_threshold_indicator: b'N',
                ipo_flag: b'Y',
                luld_reference_price_tier: b'1',
                etp_flag: b'N',
                etp_leverage_factor: 9999,
                inverse_indicator: b'N',
            }
        );
    }

    #[test]
    fn parse_stock_directory_wrong_type_byte_returns_none() {
        let mut bytes = stock_directory_bytes(
            1,
            2,
            0x0a0b_0c0d_0e0f,
            *b"AAPL    ",
            b'Q',
            b'N',
            100,
            b'Y',
            b'C',
            *b"XY",
            b'P',
            b'N',
            b'Y',
            b'1',
            b'N',
            9999,
            b'N',
        );
        bytes[0] = b'Z';
        assert!(parse_stock_directory(&bytes).is_none());
    }

    #[test]
    fn parse_stock_directory_truncated_returns_none() {
        let mut bytes = stock_directory_bytes(
            1,
            2,
            0x0a0b_0c0d_0e0f,
            *b"AAPL    ",
            b'Q',
            b'N',
            100,
            b'Y',
            b'C',
            *b"XY",
            b'P',
            b'N',
            b'Y',
            b'1',
            b'N',
            9999,
            b'N',
        );
        bytes.pop();
        assert!(parse_stock_directory(&bytes).is_none());
    }

    // ---- StockTradingActionMessage ----

    fn stock_trading_action_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        stock: [u8; 8],
        trading_state: u8,
        reserved: u8,
        reason: [u8; 4],
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(STOCK_TRADING_ACTION_MESSAGE_LEN);
        buf.push(b'H');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&stock);
        buf.push(trading_state);
        buf.push(reserved);
        buf.extend_from_slice(&reason);
        // Sanity check on the builder itself -- if this fails, the bug is in
        // the test helper, not in parse_stock_trading_action.
        assert_eq!(buf.len(), STOCK_TRADING_ACTION_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_stock_trading_action_valid() {
        // Distinct values in every field, including the adjacent single-byte
        // trading_state/reserved pair, so an offset collision between them
        // would surface as a specific mismatch rather than a coincidence.
        let timestamp = 0x0102_0304_0506u64;
        let stock = *b"MSFT    ";
        let bytes =
            stock_trading_action_bytes(0x1111, 0x2222, timestamp, stock, b'T', 0x00, *b"REG1");

        let parsed = parse_stock_trading_action(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            StockTradingActionMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                stock,
                trading_state: b'T',
                reserved: 0x00,
                reason: *b"REG1",
            }
        );
    }

    #[test]
    fn parse_stock_trading_action_wrong_type_byte_returns_none() {
        let mut bytes =
            stock_trading_action_bytes(1, 2, 0x0102_0304_0506, *b"MSFT    ", b'T', 0x00, *b"REG1");
        bytes[0] = b'Z';
        assert!(parse_stock_trading_action(&bytes).is_none());
    }

    #[test]
    fn parse_stock_trading_action_truncated_returns_none() {
        let mut bytes =
            stock_trading_action_bytes(1, 2, 0x0102_0304_0506, *b"MSFT    ", b'T', 0x00, *b"REG1");
        bytes.pop();
        assert!(parse_stock_trading_action(&bytes).is_none());
    }

    // ---- RegSHOShortSalePriceTestRestrictedIndicatorMessage ----

    fn reg_sho_bytes(
        locate_code: u16,
        tracking_number: u16,
        timestamp: u64,
        stock: [u8; 8],
        reg_sho_action: u8,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(REG_SHO_RESTRICTION_MESSAGE_LEN);
        buf.push(b'Y');
        buf.extend_from_slice(&locate_code.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&stock);
        buf.push(reg_sho_action);
        // Sanity check on the builder itself -- if this fails, the bug is in
        // the test helper, not in parse_reg_sho_short_sale_price_test_restricted_indicator.
        assert_eq!(buf.len(), REG_SHO_RESTRICTION_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_reg_sho_valid() {
        let timestamp = 0x0102_0304_0506u64;
        let stock = *b"TSLA    ";
        let bytes = reg_sho_bytes(0x1111, 0x2222, timestamp, stock, b'1');

        let parsed = parse_reg_sho_restriction(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            RegSHORestrictionMessage {
                locate_code: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                stock,
                reg_sho_action: b'1',
            }
        );
    }

    #[test]
    fn parse_reg_sho_wrong_type_byte_returns_none() {
        let mut bytes = reg_sho_bytes(1, 2, 0x0102_0304_0506, *b"TSLA    ", b'1');
        bytes[0] = b'Z';
        assert!(parse_reg_sho_restriction(&bytes).is_none());
    }

    #[test]
    fn parse_reg_sho_truncated_returns_none() {
        let mut bytes = reg_sho_bytes(1, 2, 0x0102_0304_0506, *b"TSLA    ", b'1');
        bytes.pop();
        assert!(parse_reg_sho_restriction(&bytes).is_none());
    }

    // ---- MarketParticipantPositionMessage ----

    fn market_participant_position_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        mpid: [u8; 4],
        stock: [u8; 8],
        primary_market_maker: u8,
        market_maker_mode: u8,
        market_participant_state: u8,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(MARKET_PARTICIPANT_MESSAGE_LEN);
        buf.push(b'L');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&mpid);
        buf.extend_from_slice(&stock);
        buf.push(primary_market_maker);
        buf.push(market_maker_mode);
        buf.push(market_participant_state);
        // Sanity check on the builder itself -- if this fails, the bug is in
        // the test helper, not in parse_market_participant_position.
        assert_eq!(buf.len(), MARKET_PARTICIPANT_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_market_participant_position_valid() {
        // mpid and stock are adjacent 4-byte/8-byte fields -- distinct values
        // in both catch an offset slip between them, same reasoning as the
        // trading_state/reserved pair in StockTradingAction.
        let timestamp = 0x0102_0304_0506u64;
        let mpid = *b"ABCD";
        let stock = *b"GOOG    ";
        let bytes = market_participant_position_bytes(
            0x1111, 0x2222, timestamp, mpid, stock, b'Y', b'1', b'A',
        );

        let parsed = parse_market_participant_position(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            MarketParticipantPositionMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                mpid,
                stock,
                primary_market_maker: b'Y',
                market_maker_mode: b'1',
                market_participant_state: b'A',
            }
        );
    }

    #[test]
    fn parse_market_participant_position_wrong_type_byte_returns_none() {
        let mut bytes = market_participant_position_bytes(
            1,
            2,
            0x0102_0304_0506,
            *b"ABCD",
            *b"GOOG    ",
            b'Y',
            b'1',
            b'A',
        );
        bytes[0] = b'Z';
        assert!(parse_market_participant_position(&bytes).is_none());
    }

    #[test]
    fn parse_market_participant_position_truncated_returns_none() {
        let mut bytes = market_participant_position_bytes(
            1,
            2,
            0x0102_0304_0506,
            *b"ABCD",
            *b"GOOG    ",
            b'Y',
            b'1',
            b'A',
        );
        bytes.pop();
        assert!(parse_market_participant_position(&bytes).is_none());
    }

    // ---- MWCBDeclineLevelMessage ----

    fn mwcb_decline_level_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        level1: u64,
        level2: u64,
        level3: u64,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(MWCB_DECLINE_LEVEL_MESSAGE_LEN);
        buf.push(b'V');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&level1.to_be_bytes());
        buf.extend_from_slice(&level2.to_be_bytes());
        buf.extend_from_slice(&level3.to_be_bytes());
        // Sanity check on the builder itself -- if this fails, the bug is in
        // the test helper, not in parse_mwcb_decline_level.
        assert_eq!(buf.len(), MWCB_DECLINE_LEVEL_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_mwcb_decline_level_valid() {
        // Three adjacent 8-byte fields -- distinct values in each catch an
        // offset slip between level1/level2/level3, same reasoning as the
        // mpid/stock pair in MarketParticipantPosition.
        let timestamp = 0x0102_0304_0506u64;
        let (level1, level2, level3) = (1_000_000u64, 2_000_000u64, 3_000_000u64);
        let bytes = mwcb_decline_level_bytes(0x1111, 0x2222, timestamp, level1, level2, level3);

        let parsed = parse_mwcb_decline_level(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            MWCBDeclineLevelMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                level1,
                level2,
                level3,
            }
        );
    }

    #[test]
    fn parse_mwcb_decline_level_wrong_type_byte_returns_none() {
        let mut bytes =
            mwcb_decline_level_bytes(1, 2, 0x0102_0304_0506, 1_000_000, 2_000_000, 3_000_000);
        bytes[0] = b'Z';
        assert!(parse_mwcb_decline_level(&bytes).is_none());
    }

    #[test]
    fn parse_mwcb_decline_level_truncated_returns_none() {
        let mut bytes =
            mwcb_decline_level_bytes(1, 2, 0x0102_0304_0506, 1_000_000, 2_000_000, 3_000_000);
        bytes.pop();
        assert!(parse_mwcb_decline_level(&bytes).is_none());
    }

    // ---- MWCBStatusMessage ----

    fn mwcb_status_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        breached_level: u8,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(MWCB_STATUS_MESSAGE_LEN);
        buf.push(b'W');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.push(breached_level);
        assert_eq!(buf.len(), MWCB_STATUS_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_mwcb_status_valid() {
        let timestamp = 0x0102_0304_0506u64;
        let bytes = mwcb_status_bytes(0x1111, 0x2222, timestamp, b'1');

        let parsed = parse_mwcb_status(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            MWCBStatusMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                breached_level: b'1',
            }
        );
    }

    #[test]
    fn parse_mwcb_status_wrong_type_byte_returns_none() {
        let mut bytes = mwcb_status_bytes(1, 2, 0x0102_0304_0506, b'1');
        bytes[0] = b'Z';
        assert!(parse_mwcb_status(&bytes).is_none());
    }

    #[test]
    fn parse_mwcb_status_truncated_returns_none() {
        let mut bytes = mwcb_status_bytes(1, 2, 0x0102_0304_0506, b'1');
        bytes.pop();
        assert!(parse_mwcb_status(&bytes).is_none());
    }

    // ---- IPOQuotingPeriodUpdateMessage ----

    fn ipo_quoting_period_update_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        stock: [u8; 8],
        ipo_quotation_release_time: u32,
        ipo_quotation_release_qualifier: u8,
        ipo_price: u32,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(IPO_QUOTATION_REQUEST_MESSAGE_LEN);
        buf.push(b'K');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&stock);
        buf.extend_from_slice(&ipo_quotation_release_time.to_be_bytes());
        buf.push(ipo_quotation_release_qualifier);
        buf.extend_from_slice(&ipo_price.to_be_bytes());
        assert_eq!(buf.len(), IPO_QUOTATION_REQUEST_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_ipo_quoting_period_update_valid() {
        let timestamp = 0x0102_0304_0506u64;
        let stock = *b"ABCD    ";
        let bytes = ipo_quoting_period_update_bytes(
            0x1111, 0x2222, timestamp, stock, 34_200, b'A', 200_000,
        );

        let parsed = parse_ipo_quoting_period_update(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            IPOQuotingPeriodUpdateMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                stock,
                ipo_quotation_release_time: 34_200,
                ipo_quotation_release_qualifier: b'A',
                ipo_price: 200_000,
            }
        );
    }

    #[test]
    fn parse_ipo_quoting_period_update_wrong_type_byte_returns_none() {
        let mut bytes = ipo_quoting_period_update_bytes(
            1,
            2,
            0x0102_0304_0506,
            *b"ABCD    ",
            34_200,
            b'A',
            200_000,
        );
        bytes[0] = b'Z';
        assert!(parse_ipo_quoting_period_update(&bytes).is_none());
    }

    #[test]
    fn parse_ipo_quoting_period_update_truncated_returns_none() {
        let mut bytes = ipo_quoting_period_update_bytes(
            1,
            2,
            0x0102_0304_0506,
            *b"ABCD    ",
            34_200,
            b'A',
            200_000,
        );
        bytes.pop();
        assert!(parse_ipo_quoting_period_update(&bytes).is_none());
    }

    // ---- LULDAuctionCollarMessage ----

    fn luld_auction_collar_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        stock: [u8; 8],
        auction_collar_reference_price: u32,
        upper_auction_collar_price: u32,
        lower_auction_collar_price: u32,
        auction_collar_extension: u32,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(LULD_ACTION_MESSAGE_LEN);
        buf.push(b'J');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&stock);
        buf.extend_from_slice(&auction_collar_reference_price.to_be_bytes());
        buf.extend_from_slice(&upper_auction_collar_price.to_be_bytes());
        buf.extend_from_slice(&lower_auction_collar_price.to_be_bytes());
        buf.extend_from_slice(&auction_collar_extension.to_be_bytes());
        assert_eq!(buf.len(), LULD_ACTION_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_luld_action_collar_valid() {
        // Four adjacent 4-byte fields -- distinct values in each catch an
        // offset slip between any pair of them.
        let timestamp = 0x0102_0304_0506u64;
        let stock = *b"EFGH    ";
        let bytes = luld_auction_collar_bytes(
            0x1111, 0x2222, timestamp, stock, 100_000, 110_000, 90_000, 2,
        );

        let parsed = parse_luld_action_collar(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            LULDAuctionCollarMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                stock,
                auction_collar_reference_price: 100_000,
                upper_auction_collar_price: 110_000,
                lower_auction_collar_price: 90_000,
                auction_collar_extension: 2,
            }
        );
    }

    #[test]
    fn parse_luld_action_collar_wrong_type_byte_returns_none() {
        let mut bytes = luld_auction_collar_bytes(
            1,
            2,
            0x0102_0304_0506,
            *b"EFGH    ",
            100_000,
            110_000,
            90_000,
            2,
        );
        bytes[0] = b'Z';
        assert!(parse_luld_action_collar(&bytes).is_none());
    }

    #[test]
    fn parse_luld_action_collar_truncated_returns_none() {
        let mut bytes = luld_auction_collar_bytes(
            1,
            2,
            0x0102_0304_0506,
            *b"EFGH    ",
            100_000,
            110_000,
            90_000,
            2,
        );
        bytes.pop();
        assert!(parse_luld_action_collar(&bytes).is_none());
    }

    // ---- OperationalHaltMessage ----

    fn operational_halt_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        stock: [u8; 8],
        market_code: u8,
        operational_halt_action: u8,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(OPERATIONAL_HALT_MESSAGE_LEN);
        buf.push(b'h');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&stock);
        buf.push(market_code);
        buf.push(operational_halt_action);
        assert_eq!(buf.len(), OPERATIONAL_HALT_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_operational_halt_valid() {
        let timestamp = 0x0102_0304_0506u64;
        let stock = *b"IJKL    ";
        let bytes = operational_halt_bytes(0x1111, 0x2222, timestamp, stock, b'Q', b'H');

        let parsed = parse_operational_halt(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            OperationalHaltMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                stock,
                market_code: b'Q',
                operational_halt_action: b'H',
            }
        );
    }

    #[test]
    fn parse_operational_halt_wrong_type_byte_returns_none() {
        let mut bytes = operational_halt_bytes(1, 2, 0x0102_0304_0506, *b"IJKL    ", b'Q', b'H');
        bytes[0] = b'Z';
        assert!(parse_operational_halt(&bytes).is_none());
    }

    #[test]
    fn parse_operational_halt_truncated_returns_none() {
        let mut bytes = operational_halt_bytes(1, 2, 0x0102_0304_0506, *b"IJKL    ", b'Q', b'H');
        bytes.pop();
        assert!(parse_operational_halt(&bytes).is_none());
    }

    // ---- AddOrderMessage ----

    fn add_order_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        order_reference_number: u64,
        buy_sell_indicator: u8,
        shares: u32,
        stock: [u8; 8],
        price: u32,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(ADD_ORDER_MESSAGE_LEN);
        buf.push(b'A');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&order_reference_number.to_be_bytes());
        buf.push(buy_sell_indicator);
        buf.extend_from_slice(&shares.to_be_bytes());
        buf.extend_from_slice(&stock);
        buf.extend_from_slice(&price.to_be_bytes());
        assert_eq!(buf.len(), ADD_ORDER_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_add_order_valid() {
        let timestamp = 0x0102_0304_0506u64;
        let order_reference_number = 0x1122_3344_5566_7788u64;
        let stock = *b"NVDA    ";
        let bytes = add_order_bytes(
            0x1111,
            0x2222,
            timestamp,
            order_reference_number,
            b'B',
            500,
            stock,
            1_425_000,
        );

        let parsed = parse_add_order(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            AddOrderMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                order_reference_number,
                buy_sell_indicator: b'B',
                shares: 500,
                stock,
                price: 1_425_000,
            }
        );
    }

    #[test]
    fn parse_add_order_wrong_type_byte_returns_none() {
        let mut bytes = add_order_bytes(
            1,
            2,
            0x0102_0304_0506,
            0x1122_3344_5566_7788,
            b'B',
            500,
            *b"NVDA    ",
            1_425_000,
        );
        bytes[0] = b'Z';
        assert!(parse_add_order(&bytes).is_none());
    }

    #[test]
    fn parse_add_order_truncated_returns_none() {
        let mut bytes = add_order_bytes(
            1,
            2,
            0x0102_0304_0506,
            0x1122_3344_5566_7788,
            b'B',
            500,
            *b"NVDA    ",
            1_425_000,
        );
        bytes.pop();
        assert!(parse_add_order(&bytes).is_none());
    }

    // ---- AddOrderMPIDAttributionMessage ----

    #[allow(clippy::too_many_arguments)] // test-only builder, mirrors the wire format 1:1
    fn add_order_mpid_attribution_bytes(
        stock_locate: u16,
        tracking_number: u16,
        timestamp: u64,
        order_reference_number: u64,
        buy_sell_indicator: u8,
        shares: u32,
        stock: [u8; 8],
        price: u32,
        attribution: [u8; 4],
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN);
        buf.push(b'F');
        buf.extend_from_slice(&stock_locate.to_be_bytes());
        buf.extend_from_slice(&tracking_number.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
        buf.extend_from_slice(&order_reference_number.to_be_bytes());
        buf.push(buy_sell_indicator);
        buf.extend_from_slice(&shares.to_be_bytes());
        buf.extend_from_slice(&stock);
        buf.extend_from_slice(&price.to_be_bytes());
        buf.extend_from_slice(&attribution);
        assert_eq!(buf.len(), ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN);
        buf
    }

    #[test]
    fn parse_add_order_mpid_attribution_valid() {
        let timestamp = 0x0102_0304_0506u64;
        let order_reference_number = 0x1122_3344_5566_7788u64;
        let stock = *b"AMD     ";
        let bytes = add_order_mpid_attribution_bytes(
            0x1111,
            0x2222,
            timestamp,
            order_reference_number,
            b'S',
            300,
            stock,
            850_000,
            *b"EDGX",
        );

        let parsed = parse_add_order_mpid_attribution(&bytes).expect("should parse");

        assert_eq!(
            parsed,
            AddOrderMPIDAttributionMessage {
                stock_locate: 0x1111,
                tracking_number: 0x2222,
                timestamp,
                order_reference_number,
                buy_sell_indicator: b'S',
                shares: 300,
                stock,
                price: 850_000,
                attribution: *b"EDGX",
            }
        );
    }

    #[test]
    fn parse_add_order_mpid_attribution_wrong_type_byte_returns_none() {
        let mut bytes = add_order_mpid_attribution_bytes(
            1,
            2,
            0x0102_0304_0506,
            0x1122_3344_5566_7788,
            b'S',
            300,
            *b"AMD     ",
            850_000,
            *b"EDGX",
        );
        bytes[0] = b'Z';
        assert!(parse_add_order_mpid_attribution(&bytes).is_none());
    }

    #[test]
    fn parse_add_order_mpid_attribution_truncated_returns_none() {
        let mut bytes = add_order_mpid_attribution_bytes(
            1,
            2,
            0x0102_0304_0506,
            0x1122_3344_5566_7788,
            b'S',
            300,
            *b"AMD     ",
            850_000,
            *b"EDGX",
        );
        bytes.pop();
        assert!(parse_add_order_mpid_attribution(&bytes).is_none());
    }
}
// </Generated by Claude.ai>

//! Hand-transcribed golden vectors.
//!
//! Every byte array below was written out from the ITCH 5.0 specification's
//! field-offset tables, **not** produced by this crate's builder helpers. That
//! makes them independent of the parser's own understanding of the layout: if
//! a field offset drifts, these fail while builder-based tests would still
//! pass.

use itch_parser::itch::{
    AddOrderMessage, BuySell, CrossTradeMessage, CrossType, EventCode, MWCBDeclineLevelMessage,
    MessageType, ParsedMessage, Price, Symbol, SystemEventMessage, parse_message,
};

fn sym(bytes: [u8; 8]) -> Symbol {
    Symbol::from_bytes(bytes).expect("vector symbols are valid ASCII")
}

#[test]
fn system_event_vector_from_spec_offsets() {
    // Message Type "S" @0, Stock Locate @1 (2), Tracking Number @3 (2),
    // Timestamp @5 (6), Event Code @11 (1).
    let bytes: [u8; 12] = [
        0x53, // 'S'
        0x00, 0x01, // stock locate 1
        0x00, 0x02, // tracking number 2
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, // timestamp
        0x4F, // 'O' start of messages
    ];

    assert_eq!(
        parse_message(&bytes),
        Ok(ParsedMessage::SystemEvent(SystemEventMessage {
            stock_locate: 1,
            tracking_number: 2,
            timestamp: 0x0001_0203_0405,
            event_code: EventCode::StartOfMessages,
        }))
    );
}

#[test]
fn add_order_vector_from_spec_offsets() {
    // Order Reference Number @11 (8), Buy/Sell @19 (1), Shares @20 (4),
    // Stock @24 (8), Price @32 (4).
    let bytes: [u8; 36] = [
        0x41, // 'A'
        0x00, 0x03, // stock locate 3
        0x00, 0x04, // tracking number 4
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, // timestamp
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, // order reference
        0x42, // 'B' buy
        0x00, 0x00, 0x00, 0x64, // 100 shares
        0x41, 0x41, 0x50, 0x4C, 0x20, 0x20, 0x20, 0x20, // "AAPL    "
        0x00, 0x15, 0xBE, 0x68, // price 1_425_000 (x1e-4)
    ];

    let parsed = parse_message(&bytes).expect("vector should parse");
    let ParsedMessage::AddOrder(order) = parsed else {
        panic!("expected an AddOrder, got {parsed:?}");
    };

    assert_eq!(order.stock_locate, 3);
    assert_eq!(order.tracking_number, 4);
    assert_eq!(order.timestamp, 0x0001_0203_0405);
    assert_eq!(order.order_reference_number, 0x0000_0001_0000_0002);
    assert_eq!(order.buy_sell_indicator, BuySell::Buy);
    assert_eq!(order.shares, 100);
    assert_eq!(order.stock, sym(*b"AAPL    "));
    assert_eq!(order.stock.as_str(), "AAPL");
    // 1_425_000 in Price (4) is 142.5000, which is 14_250_000_000 in 1e-8.
    assert_eq!(order.price.as_x10_8(), 14_250_000_000);
    assert!((order.price.as_f64() - 142.5).abs() < 1e-9);
}

#[test]
fn cross_trade_vector_has_an_8_byte_share_count() {
    // Shares @11 (8) — not 4 like every other share field — Stock @19 (8),
    // Cross Price @27 (4), Match Number @31 (8), Cross Type @39 (1).
    let bytes: [u8; 40] = [
        0x51, // 'Q'
        0x00, 0x05, // stock locate 5
        0x00, 0x06, // tracking number 6
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, // timestamp
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, // 4096 shares
        0x41, 0x41, 0x50, 0x4C, 0x20, 0x20, 0x20, 0x20, // "AAPL    "
        0x00, 0x0F, 0x42, 0x40, // cross price 1_000_000
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, // match number 7
        0x4F, // 'O' opening cross
    ];

    assert_eq!(
        parse_message(&bytes),
        Ok(ParsedMessage::CrossTrade(CrossTradeMessage {
            stock_locate: 5,
            tracking_number: 6,
            timestamp: 0x0001_0203_0405,
            shares: 4096,
            stock: sym(*b"AAPL    "),
            cross_price: Price::from_x10_4(1_000_000),
            match_number: 7,
            cross_type: CrossType::Opening,
        }))
    );
}

#[test]
fn mwcb_decline_level_vector_uses_the_price_8_scale() {
    // Level 1 @11 (8), Level 2 @19 (8), Level 3 @27 (8) — the only
    // `Price (8)` fields in the protocol.
    let bytes: [u8; 35] = [
        0x56, // 'V'
        0x00, 0x00, // stock locate 0
        0x00, 0x08, // tracking number 8
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, // timestamp
        0x00, 0x00, 0x00, 0x37, 0xE1, 0x1D, 0x60, 0x00, // 2400.00000000
        0x00, 0x00, 0x00, 0x6F, 0xC2, 0x3A, 0xC0, 0x00, // 4800.00000000
        0x00, 0x00, 0x00, 0xA7, 0xA3, 0x58, 0x20, 0x00, // 7200.00000000
    ];

    let parsed = parse_message(&bytes).expect("vector should parse");
    let ParsedMessage::MWCBDeclineLevel(mwcb) = parsed else {
        panic!("expected an MWCBDeclineLevel, got {parsed:?}");
    };

    assert_eq!(mwcb.level1.as_x10_8(), 240_000_000_000);
    for (price, whole) in [
        (mwcb.level1, 2400.0),
        (mwcb.level2, 4800.0),
        (mwcb.level3, 7200.0),
    ] {
        assert!((price.as_f64() - whole).abs() < 1e-9);
    }
    // A scale mix-up would put this at 24_000_000.0 instead of 2400.0.
    assert_eq!(
        MWCBDeclineLevelMessage {
            stock_locate: 0,
            tracking_number: 8,
            timestamp: 0x0001_0203_0405,
            level1: Price::from_x10_8(240_000_000_000).unwrap(),
            level2: Price::from_x10_8(480_000_000_000).unwrap(),
            level3: Price::from_x10_8(720_000_000_000).unwrap(),
        },
        mwcb
    );
}

#[test]
fn a_hand_built_message_length_matches_the_spec_table() {
    // Guards the constants themselves: the spec's offset tables imply these
    // totals, and `add_order_vector_from_spec_offsets` above indexes a
    // 36-byte array for 'A', so a drift here would be caught there too.
    let expected: &[(MessageType, usize)] = &[
        (MessageType::SystemEvent, 12),
        (MessageType::StockDirectory, 39),
        (MessageType::StockTradingAction, 25),
        (MessageType::RegSHORestriction, 20),
        (MessageType::MarketParticipantPosition, 26),
        (MessageType::MWCBDeclineLevel, 35),
        (MessageType::MWCBStatus, 12),
        (MessageType::IPOQuotingPeriodUpdate, 28),
        (MessageType::LULDAuctionCollar, 35),
        (MessageType::OperationalHalt, 21),
        (MessageType::AddOrder, 36),
        (MessageType::AddOrderMPIDAttribution, 40),
        (MessageType::OrderExecuted, 31),
        (MessageType::OrderExecutedPrice, 36),
        (MessageType::OrderCancel, 23),
        (MessageType::OrderDelete, 19),
        (MessageType::OrderReplace, 35),
        (MessageType::Trade, 44),
        (MessageType::CrossTrade, 40),
        (MessageType::BrokenTrade, 19),
        (MessageType::NOII, 50),
        (MessageType::RPII, 20),
        (MessageType::DLCRPriceDiscovery, 48),
    ];
    assert_eq!(expected.len(), MessageType::ALL.len());
    for (message_type, len) in expected {
        assert_eq!(message_type.message_len(), *len, "{message_type:?}");
    }

    let _ = AddOrderMessage {
        stock_locate: 0,
        tracking_number: 0,
        timestamp: 0,
        order_reference_number: 0,
        buy_sell_indicator: BuySell::Buy,
        shares: 0,
        stock: sym(*b"        "),
        price: Price::default(),
    };
}

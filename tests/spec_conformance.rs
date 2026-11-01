//! Conformance tests derived from the Nasdaq TotalView-ITCH 5.0 specification
//! rather than from this crate's own byte builders.
//!
//! The unit tests inside `src/itch.rs` build messages with helpers that mirror
//! the parser's offsets, so a systematic misreading of the spec would pass
//! them. The tables here are transcribed from the specification's per-message
//! offset tables, which makes offset omissions and overlaps — the class of bug
//! that has bitten this codebase before — fail loudly.

use itch_parser::itch::{MessageType, ParseError, parse_message};
use itch_parser::source::{BareItchSource, FeedSource};

/// `(field_name, offset, width)` for one message, from the ITCH 5.0 spec.
type Layout = &'static [(&'static str, usize, usize)];

/// Field layouts transcribed from the specification's offset tables.
#[allow(clippy::too_many_lines)] // one arm per message type; splitting it would hide drift
const fn layout(message_type: MessageType) -> Layout {
    match message_type {
        MessageType::SystemEvent => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("event_code", 11, 1),
        ],
        MessageType::StockDirectory => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("market_category", 19, 1),
            ("financial_status_indicator", 20, 1),
            ("round_lot_size", 21, 4),
            ("round_lots_only", 25, 1),
            ("issue_classification", 26, 1),
            ("issue_sub_type", 27, 2),
            ("authenticity", 29, 1),
            ("short_sale_threshold_indicator", 30, 1),
            ("ipo_flag", 31, 1),
            ("luld_reference_price_tier", 32, 1),
            ("etp_flag", 33, 1),
            ("etp_leverage_factor", 34, 4),
            ("inverse_indicator", 38, 1),
        ],
        MessageType::StockTradingAction => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("trading_state", 19, 1),
            ("reserved", 20, 1),
            ("reason", 21, 4),
        ],
        MessageType::RegSHORestriction => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("reg_sho_action", 19, 1),
        ],
        MessageType::MarketParticipantPosition => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("mpid", 11, 4),
            ("stock", 15, 8),
            ("primary_market_maker", 23, 1),
            ("market_maker_mode", 24, 1),
            ("market_participant_state", 25, 1),
        ],
        MessageType::MWCBDeclineLevel => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("level_1", 11, 8),
            ("level_2", 19, 8),
            ("level_3", 27, 8),
        ],
        MessageType::MWCBStatus => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("breached_level", 11, 1),
        ],
        MessageType::IPOQuotingPeriodUpdate => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("ipo_quotation_release_time", 19, 4),
            ("ipo_quotation_release_qualifier", 23, 1),
            ("ipo_price", 24, 4),
        ],
        MessageType::LULDAuctionCollar => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("auction_collar_reference_price", 19, 4),
            ("upper_auction_collar_price", 23, 4),
            ("lower_auction_collar_price", 27, 4),
            ("auction_collar_extension", 31, 4),
        ],
        MessageType::OperationalHalt => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("market_code", 19, 1),
            ("operational_halt_action", 20, 1),
        ],
        MessageType::AddOrder => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
            ("buy_sell_indicator", 19, 1),
            ("shares", 20, 4),
            ("stock", 24, 8),
            ("price", 32, 4),
        ],
        MessageType::AddOrderMPIDAttribution => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
            ("buy_sell_indicator", 19, 1),
            ("shares", 20, 4),
            ("stock", 24, 8),
            ("price", 32, 4),
            ("attribution", 36, 4),
        ],
        MessageType::OrderExecuted => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
            ("executed_shares", 19, 4),
            ("match_number", 23, 8),
        ],
        MessageType::OrderExecutedPrice => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
            ("executed_shares", 19, 4),
            ("match_number", 23, 8),
            ("printable", 31, 1),
            ("execution_price", 32, 4),
        ],
        MessageType::OrderCancel => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
            ("cancelled_shares", 19, 4),
        ],
        MessageType::OrderDelete => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
        ],
        MessageType::OrderReplace => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("original_order_reference_number", 11, 8),
            ("new_order_reference_number", 19, 8),
            ("shares", 27, 4),
            ("price", 31, 4),
        ],
        MessageType::Trade => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("order_reference_number", 11, 8),
            ("buy_sell_indicator", 19, 1),
            ("shares", 20, 4),
            ("stock", 24, 8),
            ("price", 32, 4),
            ("match_number", 36, 8),
        ],
        MessageType::CrossTrade => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("shares", 11, 8),
            ("stock", 19, 8),
            ("cross_price", 27, 4),
            ("match_number", 31, 8),
            ("cross_type", 39, 1),
        ],
        MessageType::BrokenTrade => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("match_number", 11, 8),
        ],
        MessageType::NOII => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("paired_shares", 11, 8),
            ("imbalance_shares", 19, 8),
            ("imbalance_direction", 27, 1),
            ("stock", 28, 8),
            ("far_price", 36, 4),
            ("near_price", 40, 4),
            ("current_reference_price", 44, 4),
            ("cross_type", 48, 1),
            ("price_variation_indicator", 49, 1),
        ],
        MessageType::RPII => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("interest_flag", 19, 1),
        ],
        MessageType::DLCRPriceDiscovery => &[
            ("message_type", 0, 1),
            ("stock_locate", 1, 2),
            ("tracking_number", 3, 2),
            ("timestamp", 5, 6),
            ("stock", 11, 8),
            ("open_eligibility_status", 19, 1),
            ("minimum_allowable_price", 20, 4),
            ("maximum_allowable_price", 24, 4),
            ("near_execution_price", 28, 4),
            ("near_execution_time", 32, 8),
            ("lower_price_range_collar", 40, 4),
            ("upper_price_range_collar", 44, 4),
        ],
    }
}

#[test]
fn spec_field_layouts_tile_every_message_exactly() {
    for message_type in MessageType::ALL {
        let fields = layout(message_type);
        let declared = message_type.message_len();

        assert_eq!(
            fields[0],
            ("message_type", 0, 1),
            "{message_type:?} must start with the type byte"
        );

        let mut expected_offset = 0;
        for (name, offset, width) in fields {
            assert_eq!(
                *offset, expected_offset,
                "{message_type:?} field {name:?} starts at {offset} but the previous field ended at {expected_offset} \
                 (an overlap or a gap in the layout)"
            );
            assert!(*width > 0, "{message_type:?} field {name:?} has zero width");
            expected_offset += width;
        }

        assert_eq!(
            expected_offset, declared,
            "{message_type:?} fields sum to {expected_offset} bytes but `message_len` says {declared}"
        );
    }
}

#[test]
fn spec_field_layouts_are_ordered_and_unique() {
    for message_type in MessageType::ALL {
        let fields = layout(message_type);
        for pair in fields.windows(2) {
            assert!(
                pair[0].1 < pair[1].1,
                "{message_type:?} fields are not in ascending offset order: {:?}",
                (pair[0], pair[1])
            );
            assert!(
                pair[0].0 != pair[1].0,
                "{message_type:?} has a duplicate field name {:?}",
                pair[0].0
            );
        }
    }
}

#[test]
fn every_type_rejects_every_wrong_length_without_panicking() {
    for message_type in MessageType::ALL {
        let expected_len = message_type.message_len();
        for len in 0..=64usize {
            if len == expected_len {
                continue;
            }
            let mut bytes = vec![0xAAu8; len];
            if len > 0 {
                bytes[0] = message_type.as_byte();
            }
            match parse_message(&bytes) {
                Err(
                    ParseError::WrongLength { .. }
                    | ParseError::UnknownType(_)
                    | ParseError::WrongType { .. },
                ) => {}
                other => panic!(
                    "{message_type:?} with length {len} (expected {expected_len}) should be a length error, got {other:?}"
                ),
            }
        }
    }
}

#[test]
fn a_zeroed_message_of_the_right_length_never_panics() {
    for message_type in MessageType::ALL {
        let mut bytes = vec![0u8; message_type.message_len()];
        bytes[0] = message_type.as_byte();
        // Whether it parses is irrelevant here; not panicking on a
        // structurally valid but semantically empty message is the point.
        let _ = parse_message(&bytes);
    }
}

/// Runs a full parse over any real Nasdaq capture dropped into `tests/data/`.
///
/// Nasdaq historical TotalView-ITCH files (for example
/// `01302019.NASDAQ_ITCH50.gz`, gunzipped) are a stream of 2-byte
/// big-endian length prefixes, each followed by one message. This test is
/// skipped when no such file is present, so the suite stays hermetic.
#[test]
fn real_capture_parses_without_errors() {
    let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let Ok(entries) = std::fs::read_dir(&data_dir) else {
        eprintln!("no tests/data directory; skipping real-capture conformance");
        return;
    };

    let mut checked_any = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let is_capture = file_name.contains("ITCH50") || extension.eq_ignore_ascii_case("itch");
        if !is_capture {
            continue;
        }
        checked_any = true;

        let mut source = BareItchSource::open(&path).expect("capture should open");
        let mut parsed = 0u64;
        while let Some(feed) = source.next().expect("capture should read") {
            let message = feed.message.expect("a bare feed only yields messages");
            parse_message(message).unwrap_or_else(|error| {
                panic!(
                    "{}: message {parsed} failed to parse with {error:?}: {:02x?}",
                    path.display(),
                    message
                )
            });
            parsed += 1;
        }
        assert!(parsed > 0, "{} produced no messages", path.display());
        eprintln!("{}: parsed {parsed} messages", path.display());
    }

    if !checked_any {
        eprintln!(
            "no *ITCH50 file in {}; skipping real-capture conformance",
            data_dir.display()
        );
    }
}

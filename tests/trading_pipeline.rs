//! End-to-end test of the trading stack over a real `MoldUDP64` stream.
//!
//! The fixture is synthetic, but every message is laid out per the ITCH 5.0
//! spec and travels through the same packet framing, session tracking, decoder,
//! registry, book, alpha, and gateway that a live feed would use. This is the
//! test that catches a break *between* layers, which the per-module unit tests
//! cannot see.

use std::ops::ControlFlow;

use itch_parser::alpha::{AlphaConfig, AlphaStats, ImbalanceAlpha, SignalReason};
use itch_parser::book::BookSet;
use itch_parser::gateway::{FillEvent, GatewayConfig, GatewayEvent, MockGateway};
use itch_parser::itch::{BuySell, Price, parse_message};
use itch_parser::reference::Registry;
use itch_parser::source::{
    FeedEvent, FeedStats, FilePacketSource, MoldUdp64Source, write_packets_to_file,
};
use itch_parser::{build_end_of_session_packet, build_packet, run_feed};

const SESSION: [u8; 10] = *b"TEST000001";
const LOCATE: u16 = 1;
const SYMBOL: [u8; 8] = *b"AAPL    ";
const OPEN_NS: u64 = 34_200_000_000_000;
const ONE_SECOND: u64 = 1_000_000_000;

fn push_header(buf: &mut Vec<u8>, stock_locate: u16, timestamp: u64) {
    buf.extend_from_slice(&stock_locate.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
}

fn system_event(timestamp: u64, event_code: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12);
    buf.push(b'S');
    push_header(&mut buf, 0, timestamp);
    buf.push(event_code);
    buf
}

fn stock_directory(timestamp: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(39);
    buf.push(b'R');
    push_header(&mut buf, LOCATE, timestamp);
    buf.extend_from_slice(&SYMBOL);
    buf.extend_from_slice(b"QN");
    buf.extend_from_slice(&100u32.to_be_bytes());
    buf.push(b'N');
    buf.push(b'C');
    buf.extend_from_slice(b"  ");
    buf.push(b'P');
    buf.push(b'N');
    buf.push(b'N');
    buf.push(b'1');
    buf.push(b'N');
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.push(b'N');
    assert_eq!(buf.len(), 39);
    buf
}

fn add_order(timestamp: u64, reference: u64, side: u8, shares: u32, cents: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(36);
    buf.push(b'A');
    push_header(&mut buf, LOCATE, timestamp);
    buf.extend_from_slice(&reference.to_be_bytes());
    buf.push(side);
    buf.extend_from_slice(&shares.to_be_bytes());
    buf.extend_from_slice(&SYMBOL);
    buf.extend_from_slice(&(cents * 100).to_be_bytes());
    assert_eq!(buf.len(), 36);
    buf
}

fn order_delete(timestamp: u64, reference: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(19);
    buf.push(b'D');
    push_header(&mut buf, LOCATE, timestamp);
    buf.extend_from_slice(&reference.to_be_bytes());
    assert_eq!(buf.len(), 19);
    buf
}

fn message_cents(cents: u32) -> Price {
    Price::from_x10_4(cents * 100)
}

struct Stack {
    registry: Registry,
    books: BookSet,
    alpha: ImbalanceAlpha,
    gateway: MockGateway,
    fills: Vec<FillEvent>,
    events: Vec<FeedEvent>,
}

impl Stack {
    fn new(gateway_config: GatewayConfig) -> Self {
        Self {
            registry: Registry::new(),
            books: BookSet::new(),
            alpha: ImbalanceAlpha::new(AlphaConfig::default()),
            gateway: MockGateway::new(gateway_config),
            fills: Vec::new(),
            events: Vec::new(),
        }
    }

    fn on_message(&mut self, message: &[u8]) {
        let parsed = parse_message(message).expect("fixture messages must decode");
        self.registry.apply_message(&parsed);

        let change = self.books.apply(&parsed).expect("book updates must apply");
        if matches!(change, itch_parser::book::BookChange::Ignored) {
            return;
        }

        let timestamp = parsed.timestamp();
        let Some(top) = self.books.top_of_book(LOCATE) else {
            return;
        };

        let position = self.gateway.position().shares();
        if let Some(signal) = self.alpha.on_quote(LOCATE, &top, timestamp, position) {
            assert!(matches!(
                signal.reason,
                SignalReason::Imbalance | SignalReason::Exit
            ));
            match self.gateway.submit(signal.into(), timestamp) {
                GatewayEvent::Accepted { .. } | GatewayEvent::Rejected { .. } => {}
            }
        }

        self.fills
            .extend(self.gateway.on_quote(LOCATE, &top, timestamp));
    }
}

/// Builds the same seven-update story the demo binary replays.
fn run_fixture(stack: &mut Stack, tag: &str) -> FeedStats {
    let messages = [
        system_event(OPEN_NS, b'O'),
        stock_directory(OPEN_NS),
        add_order(OPEN_NS + ONE_SECOND, 1, b'B', 1_000, 10_000),
        add_order(OPEN_NS + 2 * ONE_SECOND, 2, b'S', 100, 10_005),
        order_delete(OPEN_NS + 3 * ONE_SECOND, 2),
        order_delete(OPEN_NS + 4 * ONE_SECOND, 1),
        add_order(OPEN_NS + 5 * ONE_SECOND, 3, b'B', 900, 10_008),
        add_order(OPEN_NS + 6 * ONE_SECOND, 4, b'S', 1_000, 10_010),
    ];

    let packets: Vec<Vec<u8>> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| build_packet(SESSION, index as u64 + 1, &[message]))
        .chain(std::iter::once(build_end_of_session_packet(
            SESSION,
            messages.len() as u64 + 1,
        )))
        .collect();

    let fixture = std::env::temp_dir().join(format!(
        "itch-parser-pipeline-test-{}-{tag}.bin",
        std::process::id()
    ));
    write_packets_to_file(&fixture, &packets).expect("fixture should write");

    let mut events = Vec::new();
    let stats = run_feed(
        MoldUdp64Source::new(FilePacketSource::open(&fixture).expect("fixture should open")),
        |event| {
            events.push(event);
            ControlFlow::Continue(())
        },
        |message| stack.on_message(message),
    )
    .expect("feed should run");
    stack.events = events;
    std::fs::remove_file(&fixture).expect("fixture should be removable");
    stats
}

#[test]
fn the_full_stack_trades_the_imbalance_and_realises_a_profit() {
    let mut stack = Stack::new(GatewayConfig::default());
    let feed = run_fixture(&mut stack, "full-stack");

    // Transport: every message delivered, no integrity events beyond the
    // session boundary.
    assert_eq!(feed.messages, 8);
    assert_eq!(feed.gaps, 0);
    assert_eq!(feed.truncated_packets, 0);
    assert!(feed.ended);
    assert!(stack.events.contains(&FeedEvent::EndOfSession));
    assert_eq!(
        stack
            .events
            .iter()
            .filter(|event| matches!(event, FeedEvent::NewSession { .. }))
            .count(),
        1
    );

    // Ingest: the locate resolves to the symbol the directory announced.
    assert_eq!(stack.registry.symbol_str(LOCATE), Some("AAPL"));
    assert_eq!(stack.registry.len(), 1);
    assert_eq!(stack.registry.stats().inserted, 1);

    // Book: the synthetic market ends at 100.08 x 900 / 100.10 x 1000, and the
    // order and level views agree.
    let book = stack.books.book(LOCATE).expect("book should exist");
    let bid = book.best_bid().expect("bid side should exist");
    let ask = book.best_ask().expect("ask side should exist");
    assert_eq!(bid.price, message_cents(10_008));
    assert_eq!(bid.shares, 900);
    assert_eq!(ask.price, message_cents(10_010));
    assert_eq!(ask.shares, 1_000);
    assert!(book.is_consistent());
    assert!(stack.books.is_consistent());

    // Alpha: one entry on the heavy bid, one exit when the imbalance flips.
    let AlphaStats { entries, exits, .. } = stack.alpha.stats();
    assert_eq!(entries, 1);
    assert_eq!(exits, 1);

    // Gateway: bought the offer, sold the bid, flat, +$3.00.
    assert_eq!(stack.fills.len(), 2);
    assert_eq!(stack.fills[0].side, BuySell::Buy);
    assert_eq!(stack.fills[0].price, message_cents(10_005));
    assert_eq!(stack.fills[0].shares, 100);
    assert_eq!(stack.fills[1].side, BuySell::Sell);
    assert_eq!(stack.fills[1].price, message_cents(10_008));
    assert_eq!(stack.fills[1].shares, 100);

    let position = stack.gateway.position();
    assert!(position.is_flat(), "the exit must flatten the position");
    assert_eq!(position.realized_pnl(), 3 * Price::SCALE);
    assert_eq!(
        Price::from_x10_8_canonical(position.realized_pnl()).to_string(),
        "3"
    );
    assert_eq!(stack.gateway.stats().accepted, 2);
    assert_eq!(stack.gateway.stats().rejected, 0);
}

#[test]
fn a_gateway_rejection_stops_the_trade_without_corrupting_the_book() {
    // Cap single orders below the alpha's 100-share child order.
    let mut stack = Stack::new(GatewayConfig {
        max_order_shares: 50,
        ..GatewayConfig::default()
    });
    let feed = run_fixture(&mut stack, "rejection");

    assert_eq!(feed.gaps, 0);
    assert!(stack.fills.is_empty());
    assert_eq!(stack.gateway.stats().rejected, 1);
    assert_eq!(stack.gateway.stats().accepted, 0);
    assert!(stack.gateway.position().is_flat());
    // The book still reflects the feed, not our rejected orders.
    assert!(stack.books.is_consistent());
    let book = stack.books.book(LOCATE).unwrap();
    assert_eq!(book.best_bid().unwrap().shares, 900);
    assert_eq!(book.best_ask().unwrap().shares, 1_000);
}

#[test]
fn a_rejected_order_is_reported_and_nothing_is_left_working() {
    let mut gateway = MockGateway::new(GatewayConfig {
        max_position: 50,
        ..GatewayConfig::default()
    });
    let event = gateway.submit(
        itch_parser::gateway::OrderIntent {
            stock_locate: LOCATE,
            side: BuySell::Buy,
            shares: 100,
            limit_price: message_cents(10_005),
        },
        0,
    );
    assert!(matches!(event, GatewayEvent::Rejected { .. }));
    assert_eq!(gateway.open_order_count(), 0);
    assert_eq!(gateway.stats().submitted, 1);
    assert_eq!(gateway.stats().rejected, 1);
}

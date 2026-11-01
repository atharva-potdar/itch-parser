//! End-to-end demo: decode → reference data → order book → alpha → gateway.
//!
//! The fixture is synthetic but built from real ITCH message layouts, carried
//! through a real `MoldUDP64` stream. The point is to show the whole stack
//! running on the same bytes a live feed would deliver, not to simulate a
//! market. Run it with:
//!
//! ```sh
//! cargo run --bin trading_demo
//! ```

use std::ops::ControlFlow;

use itch_parser::alpha::{AlphaConfig, ImbalanceAlpha, Signal, SignalReason};
use itch_parser::book::{BookSet, TopOfBook};
use itch_parser::gateway::{GatewayConfig, GatewayEvent, GatewayStats, MockGateway};
use itch_parser::itch::{Price, parse_message};
use itch_parser::reference::Registry;
use itch_parser::source::{FeedEvent, FilePacketSource, MoldUdp64Source, write_packets_to_file};
use itch_parser::{build_end_of_session_packet, build_packet, run_feed};

const SESSION: [u8; 10] = *b"DEMO000001";
const LOCATE: u16 = 1;
const SYMBOL: [u8; 8] = *b"AAPL    ";
/// 09:30:00 in nanoseconds since midnight.
const OPEN_NS: u64 = 34_200_000_000_000;
const ONE_SECOND: u64 = 1_000_000_000;

fn push_header(buf: &mut Vec<u8>, stock_locate: u16, timestamp: u64) {
    buf.extend_from_slice(&stock_locate.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // tracking number
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
    buf.push(b'Q'); // Nasdaq Global Select
    buf.push(b'N'); // financial status: normal
    buf.extend_from_slice(&100u32.to_be_bytes()); // round lot
    buf.push(b'N'); // round lots only
    buf.push(b'C'); // common stock
    buf.extend_from_slice(b"  "); // issue sub-type
    buf.push(b'P'); // production
    buf.push(b'N'); // short sale threshold
    buf.push(b'N'); // IPO flag
    buf.push(b'1'); // LULD tier 1
    buf.push(b'N'); // ETP
    buf.extend_from_slice(&0u32.to_be_bytes()); // leverage factor
    buf.push(b'N'); // inverse
    debug_assert_eq!(buf.len(), 39);
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
    // `Price (4)` is units of 1e-4, and one cent is 100 of those units.
    buf.extend_from_slice(&(cents * 100).to_be_bytes());
    debug_assert_eq!(buf.len(), 36);
    buf
}

fn order_delete(timestamp: u64, reference: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(19);
    buf.push(b'D');
    push_header(&mut buf, LOCATE, timestamp);
    buf.extend_from_slice(&reference.to_be_bytes());
    debug_assert_eq!(buf.len(), 19);
    buf
}

struct DemoState {
    registry: Registry,
    books: BookSet,
    alpha: ImbalanceAlpha,
    gateway: MockGateway,
    fills: u64,
}

fn on_message(state: &mut DemoState, message: &[u8]) {
    let parsed = match parse_message(message) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("unparseable message: {error:?}");
            return;
        }
    };

    state.registry.apply_message(&parsed);

    let change = match state.books.apply(&parsed) {
        Ok(change) => change,
        Err(error) => {
            eprintln!("book rejected message: {error:?}");
            return;
        }
    };
    if matches!(change, itch_parser::book::BookChange::Ignored) {
        return;
    }

    let timestamp = parsed.timestamp();
    let Some(top) = state.books.top_of_book(LOCATE) else {
        return;
    };
    print_quote(&state.registry, &top, timestamp);

    // The alpha decides, the gateway risk-checks, and the fill model runs on
    // the same quote the decision was made from.
    let position = state.gateway.position().shares();
    if let Some(signal) = state.alpha.on_quote(LOCATE, &top, timestamp, position) {
        print_signal(&state.registry, &signal);
        let intent = signal.into();
        match state.gateway.submit(intent, timestamp) {
            GatewayEvent::Accepted { order_id, .. } => {
                println!("    ack: order {order_id} accepted");
            }
            GatewayEvent::Rejected { reason, .. } => {
                println!("    reject: {reason:?}");
            }
        }
    }

    // Any working order -- the one just submitted, or an older resting order --
    // is matched against the quote that has just arrived.
    for fill in state.gateway.on_quote(LOCATE, &top, timestamp) {
        state.fills += 1;
        println!(
            "    fill: order {} {} {} @ {}",
            fill.order_id,
            if fill.side == itch_parser::itch::BuySell::Buy {
                "BUY "
            } else {
                "SELL"
            },
            fill.shares,
            fill.price
        );
    }
}

#[allow(clippy::cast_precision_loss)]
fn print_quote(registry: &Registry, top: &TopOfBook, timestamp: u64) {
    let name = registry.symbol_str(LOCATE).unwrap_or("?");
    let bid = top.bid.map_or_else(
        || "   ---".to_string(),
        |level| format!("{:>8} x {}", level.price, level.shares),
    );
    let ask = top.ask.map_or_else(
        || "   ---".to_string(),
        |level| format!("{:>8} x {}", level.price, level.shares),
    );
    let imbalance = top.imbalance().unwrap_or(0.0);
    println!(
        "[{:>6.3}s] {name} {bid} | {ask}  imb={imbalance:+.3}",
        timestamp.saturating_sub(OPEN_NS) as f64 / 1e9
    );
}

fn print_signal(registry: &Registry, signal: &Signal) {
    let name = registry.symbol_str(signal.stock_locate).unwrap_or("?");
    let side = if signal.side == itch_parser::itch::BuySell::Buy {
        "BUY "
    } else {
        "SELL"
    };
    let why = match signal.reason {
        SignalReason::Imbalance => "imbalance",
        SignalReason::Exit => "exit",
    };
    println!(
        "    signal: {side} {} {name} @ {} ({why}, imb {:+.3}, edge {:+.2} bps)",
        signal.shares,
        signal.limit_price,
        signal.imbalance,
        signal.edge_bps()
    );
}

fn main() -> std::io::Result<()> {
    // A story in seven book updates: a heavy bid opens a long, then the market
    // trades up and the imbalance flips, releasing it for a small profit.
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

    let fixture = std::env::temp_dir().join("itch-parser-trading-demo.bin");
    write_packets_to_file(&fixture, &packets)?;

    let mut state = DemoState {
        registry: Registry::new(),
        books: BookSet::new(),
        alpha: ImbalanceAlpha::new(AlphaConfig::default()),
        gateway: MockGateway::new(GatewayConfig::default()),
        fills: 0,
    };

    println!(
        "decoding {} packets from {}",
        packets.len(),
        fixture.display()
    );
    let feed_stats = run_feed(
        MoldUdp64Source::new(FilePacketSource::open(&fixture)?),
        |event| match event {
            FeedEvent::EndOfSession => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        },
        |message| on_message(&mut state, message),
    )?;
    std::fs::remove_file(&fixture)?;

    let position = state.gateway.position();
    println!();
    println!("--- summary ---");
    println!(
        "symbol:     {} (locate {}, {} instruments known)",
        state.registry.symbol_str(LOCATE).unwrap_or("?"),
        LOCATE,
        state.registry.len()
    );
    println!(
        "book:       {} orders / {} levels, consistent: {}",
        state
            .books
            .book(LOCATE)
            .map_or(0, itch_parser::book::Book::order_count),
        state
            .books
            .book(LOCATE)
            .map_or(0, itch_parser::book::Book::level_count),
        state.books.is_consistent()
    );
    println!(
        "position:   {} shares, average {}, realised P&L {}",
        position.shares(),
        position.average_price(),
        Price::from_x10_8_canonical(position.realized_pnl())
    );
    println!("fills:      {}", state.fills);
    println!(
        "feed:       {:?}",
        FeedStatsSummary {
            stats: feed_stats,
            books: state.books.stats(),
            alpha: state.alpha.stats(),
            gateway: state.gateway.stats(),
        }
    );
    Ok(())
}

/// Keeps the summary on one line without dumping four structs verbatim.
struct FeedStatsSummary {
    stats: itch_parser::source::FeedStats,
    books: itch_parser::book::BookStats,
    alpha: itch_parser::alpha::AlphaStats,
    gateway: GatewayStats,
}

impl std::fmt::Debug for FeedStatsSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} packets, {} messages, {} gaps; book {} adds / {} fills; alpha {} signals; gateway {} accepted / {} rejected",
            self.stats.packets,
            self.stats.messages,
            self.stats.gaps,
            self.books.added,
            self.gateway.fills,
            self.alpha.signals,
            self.gateway.accepted,
            self.gateway.rejected
        )
    }
}

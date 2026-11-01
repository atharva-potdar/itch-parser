# itch-parser

An allocation-free decoder for Nasdaq's **TotalView-ITCH 5.0** feed, carried over
**MoldUDP64**, written in Rust — plus the layers that make it usable: reference
data, an order book, a demo alpha, and a paper-trading gateway.

The decoder turns bytes into typed messages and tells you honestly when it
could not. Everything above it is built from those messages and nothing else.

```text
PacketSource            bytes: length-prefixed file, or UDP datagram
     │
MoldUdp64Source         header → session → sequence → messages + integrity events
BareItchSource          2-byte length prefix → bare ITCH messages (historical files)
     │
parse_message           bytes → ParsedMessage                  (src/itch.rs)
     │
Registry                stock_locate ⇄ Symbol, from R messages  (src/reference.rs)
BookSet                 A/F/E/C/X/D/U → per-locate order book   (src/book.rs)
     │
ImbalanceAlpha          top-of-book imbalance → Signal          (src/alpha.rs)
MockGateway             risk checks, paper fills, P&L          (src/gateway.rs)
```

## What it does

- Decodes **all 23 ITCH 5.0 message types** into typed structs.
- Tracks `MoldUDP64` sessions, sequence numbers, gaps, heartbeats, and
  end-of-session packets.
- **Refuses to deliver past an unrecovered gap** by default, so a stateful
  consumer such as an order book cannot silently diverge.
- Reports gaps, truncated packets, and stale/duplicate packets as explicit
  events plus counters, rather than swallowing them.
- Normalises prices into a single `Price` (1e-8 units) so the `Price (4)` and
  `Price (8)` wire scales cannot be mixed by accident.
- Resolves `stock_locate` to a symbol through a registry that survives locates
  being reused and symbols moving.
- Rebuilds the displayed limit order book per locate, with an order view and a
  price-level view kept consistent.
- Ships a small, auditable order-flow alpha and a paper gateway with pre-trade
  risk checks and realised/unrealised P&L.
- Never allocates per message on the decode path, and never panics on malformed
  input.

## What it deliberately does not do

- Place real orders or speak to an exchange. `MockGateway` is a paper model.
- Claim the demo alpha is profitable. It is a readable example, not a strategy.
- Emit retransmission requests on its own — it gives you the bytes and the
  events so *you* can decide.
- Buffer out-of-order packets. Under `GapPolicy::Halt` it stops instead, which
  is safer than a half-correct reorder buffer.
- Support `SoupBinTCP`.

## Usage

### Historical Nasdaq capture

Nasdaq historical files (for example `01302019.NASDAQ_ITCH50`) are a stream of
2-byte big-endian length prefixes, each followed by one message, with no
`MoldUDP64` header.

```rust no_run
use itch_parser::itch::parse_message;
use itch_parser::source::{BareItchSource, FeedSource};

# fn main() -> std::io::Result<()> {
let mut source = BareItchSource::open("sample.NASDAQ_ITCH50")?;
while let Some(feed) = source.next()? {
    let message = feed.message.expect("a bare feed only yields messages");
    let parsed = parse_message(message).expect("conforming capture");
    println!("{parsed:?}");
}
# Ok(())
# }
```

A bare file carries no sequence numbers, so it can never report a gap: treat it
as complete by construction.

### Live `MoldUDP64` with gap recovery

```rust no_run
use itch_parser::source::{FeedEvent, MoldUdp64Source, Retransmitter, UdpPacketSource, request_missing};
use itch_parser::run_feed;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::ops::ControlFlow;

# fn main() -> std::io::Result<()> {
let source = MoldUdp64Source::new(UdpPacketSource::join_multicast(
    "239.1.1.1".parse().unwrap(),
    12345,
    Ipv4Addr::UNSPECIFIED,
)?);
let mut retransmitter =
    Retransmitter::connect(SocketAddrV4::new(Ipv4Addr::new(239, 1, 1, 1), 12346))?;

run_feed(
    source,
    |event| {
        request_missing(&event, &mut retransmitter).ok();
        if event == FeedEvent::EndOfSession {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    },
    |message| println!("{} bytes", message.len()),
)?;
# Ok(())
# }
```

Under the default `GapPolicy::Halt`, nothing past an open gap is delivered. Use
`GapPolicy::Skip` for loss-tolerant consumers.

### Rebuilding the book

```rust
use itch_parser::book::BookSet;
use itch_parser::itch::{BuySell, Price};

let mut books = BookSet::new();
let book = books.book_mut(1);
book.add(42, BuySell::Buy, 100, Price::from_x10_4(1_000_000)).unwrap();
book.add(43, BuySell::Sell, 200, Price::from_x10_4(1_000_100)).unwrap();

assert_eq!(book.best_bid().unwrap().shares, 100);
assert_eq!(book.mid().unwrap().to_string(), "100.005");
assert_eq!(book.spread().unwrap().to_string(), "0.01");
assert!(books.is_consistent());
```

`BookSet::apply` consumes decoded messages, so a feed loop is just
`parse_message` followed by `books.apply(&parsed)`. Reference data works the
same way: `Registry::apply_message(&parsed)` handles the `R` messages and
ignores everything else.

### Trading demo

```sh
cargo run --bin trading_demo
```

The demo replays a synthetic session through the real feed, registry, book,
alpha, and gateway. It buys the offer on a heavy bid, sells the bid when the
imbalance flips, and ends flat with a small realised profit. Every number in
the output is traceable to a message.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `bench-alloc` | off | Links `mimalloc` as the global allocator and enables the `bench` binary. Keep it off in library consumers. |

```sh
cargo run --release --features bench-alloc --bin bench
```

## Binaries

| Binary | Purpose |
|---|---|
| `itch-parser` | Feed demo: build fixtures, run the feed, show every event. |
| `trading_demo` | Full-stack demo: decode → reference → book → alpha → paper fills. |
| `make_fixture` | Write a `MoldUDP64` fixture file. |
| `replay` | Send a fixture over multicast UDP. |
| `receive` | Listen on multicast UDP, print events, send re-requests. |
| `bench` | Hot-path throughput measurement (requires `bench-alloc`). |

## Testing

```sh
cargo test
cargo clippy --all-targets --features bench-alloc -- -D warnings
cargo run --release --features bench-alloc --bin bench
```

The suite is split between unit tests in each module and three independent
integration suites:

- `tests/spec_conformance.rs` re-derives every message length from the ITCH 5.0
  spec's offset tables and checks that every type rejects every wrong length.
- `tests/golden_vectors.rs` asserts hand-transcribed byte arrays that were
  written from the spec, not generated by the crate's own builders.
- `tests/trading_pipeline.rs` runs a synthetic `MoldUDP64` session through every
  layer and asserts the boundaries between them: no gaps, a resolved symbol, a
  consistent book after our own fills, and the expected P&L.

Drop a real capture into `tests/data/` (see the README there) to additionally
run a full-file conformance pass.

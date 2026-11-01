use itch_parser::itch::{EventCode, ParsedMessage, parse_message};
use itch_parser::source::{FeedEvent, FilePacketSource, MoldUdp64Source, write_packets_to_file};
use itch_parser::{build_end_of_session_packet, build_packet, run_feed};
use std::ops::ControlFlow;

/// Hand-builds a minimal, real System Event message.
fn system_event_message_bytes(
    stock_locate: u16,
    tracking_number: u16,
    timestamp_ns: u64,
    event_code: u8,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12);
    buf.push(b'S');
    buf.extend_from_slice(&stock_locate.to_be_bytes());
    buf.extend_from_slice(&tracking_number.to_be_bytes());
    buf.extend_from_slice(&timestamp_ns.to_be_bytes()[2..8]);
    buf.push(event_code);
    buf
}

fn main() -> std::io::Result<()> {
    let session_id = *b"SESSION001";
    // 'O' = Start of Messages.
    let system_event = system_event_message_bytes(1, 1, 34_200_000_000_000, b'O');

    let demo_packets = vec![
        build_packet(session_id, 1, &[system_event.as_slice()]),
        build_packet(session_id, 2, &[b"NOT_REAL_ITCH"]), // intentionally unparseable
        build_packet(session_id, 10, &[system_event.as_slice()]), // gap
        build_packet(session_id, 1, &[system_event.as_slice()]), // duplicate
        build_end_of_session_packet(session_id, 12),
    ];

    let fixture_path = std::env::temp_dir().join("moldudp64_demo.bin");
    write_packets_to_file(&fixture_path, &demo_packets)?;

    let source = MoldUdp64Source::new(FilePacketSource::open(&fixture_path)?);

    let mut unparsed = 0usize;

    let stats = run_feed(
        source,
        |event| {
            println!("{event:?}");
            // Under the default `GapPolicy::Halt` nothing past a gap is
            // delivered, so the stream stays trustworthy without any special
            // handling here.
            match event {
                FeedEvent::EndOfSession => ControlFlow::Break(()),
                _ => ControlFlow::Continue(()),
            }
        },
        |message| match parse_message(message) {
            Ok(ParsedMessage::SystemEvent(event)) => {
                println!("  system event: {event:?}");
                if event.event_code == EventCode::EndOfMessages {
                    println!("  (end of messages)");
                }
            }
            Ok(other) => println!("  {other:?}"),
            Err(error) => {
                unparsed += 1;
                eprintln!("  unparseable ITCH message: {error:?} {message:02x?}");
            }
        },
    )?;

    println!("stats: {stats:?}");
    println!("unparsed messages: {unparsed}");

    std::fs::remove_file(&fixture_path)?;
    Ok(())
}

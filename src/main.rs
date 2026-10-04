use itch_parser::itch::parse_message;
use itch_parser::source::{FilePacketSource, write_packets_to_file};
use itch_parser::{Session, build_end_of_session_packet, build_packet, run_pipeline};

/// Hand-builds a minimal, real System Event message
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
    let system_event = system_event_message_bytes(1, 1, 34_200_000_000_000, b'O'); // 'O' = Start of Messages

    let demo_packets = vec![
        build_packet(session_id, 1, &[system_event.as_slice()]),
        build_packet(session_id, 2, &[b"NOT_REAL_ITCH"]), // intentionally unparseable
        build_packet(session_id, 10, &[system_event.as_slice()]), // gap
        build_packet(session_id, 1, &[system_event.as_slice()]), // duplicate
        build_end_of_session_packet(session_id, 12),
    ];

    let fixture_path = std::env::temp_dir().join("moldudp64_demo.bin");
    write_packets_to_file(&fixture_path, &demo_packets)?;

    let source = FilePacketSource::open(&fixture_path)?;
    let mut session = Session::new();
    let mut unparsed_count = 0usize;

    run_pipeline(
        source,
        &mut session,
        |header, status| {
            println!(
                "seq={} count={} -> {status:?}",
                header.sequence_number, header.message_count
            );
        },
        |message| {
            if let Some(parsed) = parse_message(message) {
                println!("  {parsed:?}");
            } else {
                unparsed_count += 1;
                eprintln!("  unparseable ITCH message: {message:02x?}");
            }
        },
    )?;

    println!("unparsed messages: {unparsed_count}");

    std::fs::remove_file(&fixture_path)?;
    Ok(())
}

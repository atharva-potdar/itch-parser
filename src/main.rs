use itch_parser::source::{FilePacketSource, write_packets_to_file};
use itch_parser::{Session, build_end_of_session_packet, build_packet, run_pipeline};

fn main() -> std::io::Result<()> {
    let session_id = *b"SESSION001";

    let demo_packets = vec![
        build_packet(session_id, 1, &[b"MSG_A", b"MSG_B"]),
        build_packet(session_id, 3, &[b"MSG_C"]),
        build_packet(session_id, 10, &[b"MSG_D", b"MSG_E"]), // gap
        build_packet(session_id, 1, &[b"MSG_A", b"MSG_B"]),  // duplicate
        build_end_of_session_packet(session_id, 12),
    ];

    let fixture_path = std::env::temp_dir().join("moldudp64_demo.bin");
    write_packets_to_file(&fixture_path, &demo_packets)?;

    let source = FilePacketSource::open(&fixture_path)?;
    let mut session = Session::new();

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
            println!("  message: {}", String::from_utf8_lossy(message));
        },
    )?;

    std::fs::remove_file(&fixture_path)?;
    Ok(())
}

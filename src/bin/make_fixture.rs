use itch_parser::{build_end_of_session_packet, build_packet};
use itch_parser::source::write_packets_to_file;
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fixture.bin"));

    let session_id = *b"SESSION001";

    let packets = vec![
        build_packet(session_id, 1, &[b"MSG_A", b"MSG_B"]),
        build_packet(session_id, 3, &[b"MSG_C"]),
        build_packet(session_id, 10, &[b"MSG_D", b"MSG_E"]), // gap
        build_packet(session_id, 1, &[b"MSG_A", b"MSG_B"]),  // duplicate
        build_end_of_session_packet(session_id, 12),
    ];

    write_packets_to_file(&out_path, &packets)?;
    println!("wrote {} packets to {}", packets.len(), out_path.display());
    Ok(())
}
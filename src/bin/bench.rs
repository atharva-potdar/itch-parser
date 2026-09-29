use itch_parser::{parse_header, parse_messages, PacketStatus, Session};
use std::hint::black_box;
use std::process::exit;
use std::time::Instant;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const PACKET_COUNT: usize = 10_000_000;

fn make_hot_path_packets(session_id: [u8; 10]) -> (Vec<u8>, Vec<(usize, usize)>) {
    let mut buffer = Vec::with_capacity(PACKET_COUNT * 64);
    let mut ranges = Vec::with_capacity(PACKET_COUNT);

    let msg1 = b"AAPL0000";
    let msg2 = b"MSFT0000";

    for i in 0..PACKET_COUNT {
        let sequence_number = 1 + (i as u64) * 2;
        let start_idx = buffer.len();

        buffer.extend_from_slice(&session_id);
        buffer.extend_from_slice(&sequence_number.to_be_bytes());
        buffer.extend_from_slice(&2u16.to_be_bytes()); // message_count

        buffer.extend_from_slice(&(msg1.len() as u16).to_be_bytes());
        buffer.extend_from_slice(msg1);

        buffer.extend_from_slice(&(msg2.len() as u16).to_be_bytes());
        buffer.extend_from_slice(msg2);

        ranges.push((start_idx, buffer.len()));
    }

    (buffer, ranges)
}

fn bench_hot_path(buffer: &[u8], ranges: &[(usize, usize)]) {
    let mut session = Session::new();
    let mut messages_seen = 0u64;

    let start = Instant::now();
    for &(start_idx, end_idx) in ranges {
        let packet = &buffer[start_idx..end_idx];

        let Some((header, body)) = parse_header(black_box(packet)) else {
            continue;
        };
        let status = session.on_packet(&header);
        if matches!(status, PacketStatus::InOrder | PacketStatus::Gap { .. }) {
            parse_messages(body, header.message_count, |msg| {
                black_box(msg);
                messages_seen += 1;
            });
        }
    }
    let elapsed = start.elapsed();

    black_box(messages_seen);

    println!(
        "hot path: {} packets, {messages_seen} messages in {elapsed:.3?} ({:.1} ns/packet)",
        ranges.len(),
        elapsed.as_secs_f64() * 1e9 / f64::from(u32::try_from(ranges.len()).unwrap()),
    );
}

fn main() {
    let session_id = *b"SESSION001";
    let (buffer, ranges) = make_hot_path_packets(session_id);
    bench_hot_path(&buffer, &ranges);
    exit(0);
}
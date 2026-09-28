use itch_parser::run_pipeline;
use itch_parser::source::UdpPacketSource;
use itch_parser::Session;
use std::net::Ipv4Addr;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let multicast_addr: Ipv4Addr = args
        .next()
        .as_deref()
        .unwrap_or("239.1.1.1")
        .parse()
        .expect("invalid multicast address");
    let port: u16 = args
        .next()
        .as_deref()
        .unwrap_or("12345")
        .parse()
        .expect("invalid port");

    let source = UdpPacketSource::join_multicast(multicast_addr, port, Ipv4Addr::UNSPECIFIED)?;
    let mut session = Session::new();

    println!("listening on {multicast_addr}:{port} ...");

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

    Ok(())
}
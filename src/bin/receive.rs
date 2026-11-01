use itch_parser::itch::parse_message;
use itch_parser::source::{FeedEvent, MoldUdp64Source, UdpPacketSource};
use itch_parser::{run_feed, source::request_missing};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::ops::ControlFlow;

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
    let request_port: u16 = args
        .next()
        .as_deref()
        .unwrap_or("12346")
        .parse()
        .expect("invalid re-request port");

    let source = MoldUdp64Source::new(UdpPacketSource::join_multicast(
        multicast_addr,
        port,
        Ipv4Addr::UNSPECIFIED,
    )?);
    let mut retransmitter = itch_parser::source::Retransmitter::connect(SocketAddrV4::new(
        multicast_addr,
        request_port,
    ))?;

    println!("listening on {multicast_addr}:{port} (re-requests to :{request_port}) ...");

    let stats = run_feed(
        source,
        |event| {
            println!("{event:?}");
            if let Err(error) = request_missing(&event, &mut retransmitter) {
                eprintln!("re-request failed: {error}");
            }
            match event {
                FeedEvent::EndOfSession => ControlFlow::Break(()),
                _ => ControlFlow::Continue(()),
            }
        },
        |message| match parse_message(message) {
            Ok(parsed) => println!("  {parsed:?}"),
            Err(error) => eprintln!("  unparseable: {error:?}"),
        },
    )?;

    println!("stats: {stats:?}");
    Ok(())
}

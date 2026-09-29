use itch_parser::source::{FilePacketSource, PacketSource};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let fixture_path = args
        .next()
        .map(PathBuf::from)
        .expect("usage: replay <fixture-file> [multicast-addr] [port]");
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

    let target = SocketAddrV4::new(multicast_addr, port);

    // Ephemeral local port
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;

    // Necessary specifically for local loopback testing
    socket.set_multicast_loop_v4(true)?;

    let mut source = FilePacketSource::open(&fixture_path)?;
    let mut sent = 0usize;

    let mut buf = Vec::new();

    while let Some(received) = source.next_packet(&mut buf)? {
        let packet = &buf[..received];
        socket.send_to(packet, target)?;
        sent += 1;
        println!("sent packet {sent} ({} bytes) to {target}", packet.len());
    }

    println!("done: {sent} packets sent to {target}");
    Ok(())
}

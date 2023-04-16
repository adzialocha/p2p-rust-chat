use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use async_std::net::UdpSocket;
use async_std::stream::StreamExt;
use async_std::{channel, io, task};
use net2::unix::UnixUdpBuilderExt;
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook_async_std::Signals;
use simple_dns::rdata::{RData, TXT};
use simple_dns::{Name, Packet, PacketFlag, Question, ResourceRecord, CLASS, QCLASS, QTYPE, TYPE};

const MULTICAST_PORT: u16 = 5353;
const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

const SERVICE_NAME: &'static str = "_srv.p2p-chat.local";
const ANNOUNCEMENT_PER_SEC: u64 = 1;

enum Events {
    PeerDiscovered(SocketAddr),
    SendMessage(String),
}

fn mdns_announce_packet() -> Vec<u8> {
    let mut packet = Packet::new_query(1);

    packet.questions.push(Question::new(
        Name::new_unchecked(SERVICE_NAME),
        TYPE::TXT.into(),
        CLASS::IN.into(),
        false,
    ));

    let bytes = packet
        .build_bytes_vec()
        .expect("We've created a bad DNS packet");

    bytes
}

fn mdns_response_packet(addr: &SocketAddr) -> Vec<u8> {
    let mut packet = Packet::new_query(1);
    packet.set_flags(PacketFlag::AUTHORITATIVE_ANSWER);

    let mut record_data = TXT::new();
    let addr_str = format!("address={}", addr);
    record_data.add_string(&addr_str).unwrap();

    packet.answers.push(ResourceRecord::new(
        Name::new_unchecked(SERVICE_NAME),
        CLASS::IN.into(),
        10,
        RData::TXT(record_data),
    ));

    let bytes = packet
        .build_bytes_vec()
        .expect("We've created a bad DNS packet");

    bytes
}

enum DiscoveryMessage {
    Announcement,
    Response(SocketAddr),
}

fn parse_mdns_packet(bytes: &[u8]) -> Option<DiscoveryMessage> {
    if let Ok(packet) = Packet::parse(bytes) {
        if is_valid_mdns_announce_packet(&packet) {
            return Some(DiscoveryMessage::Announcement);
        }

        if let Some(addr) = is_valid_mdns_response_packet(&packet) {
            return Some(DiscoveryMessage::Response(addr));
        }
    }

    None
}

fn is_valid_mdns_announce_packet(packet: &Packet) -> bool {
    packet.questions.len() == 1
        && packet.questions[0].qname.to_string() == SERVICE_NAME
        && packet.questions[0].qtype == QTYPE::TYPE(TYPE::TXT)
        && packet.questions[0].qclass == QCLASS::CLASS(CLASS::IN)
        && packet.questions[0].unicast_response == false
}

fn is_valid_mdns_response_packet(packet: &Packet) -> Option<SocketAddr> {
    if packet.answers.len() == 1
        && packet.answers[0].name.to_string() == SERVICE_NAME
        && packet.answers[0].match_qtype(QTYPE::TYPE(TYPE::TXT))
        && packet.answers[0].match_qclass(QCLASS::CLASS(CLASS::IN))
    {
        // Check if TXT record exists with correct address value inside
        if let RData::TXT(record_data) = &packet.answers[0].rdata {
            if let Some(Some(addr_str)) = record_data.attributes().get("address") {
                // Try to parse address string
                return match addr_str.parse() {
                    Ok(addr) => Some(addr),
                    Err(_) => None,
                };
            }
        }
    }

    None
}

#[async_std::main]
async fn main() {
    let (tx, rx) = channel::unbounded::<Events>();

    let chat_socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.unwrap();
    let local_addr = chat_socket.local_addr().unwrap();
    println!("Listening at {}", local_addr);

    // Ask for other p2p-chat peers on the network via mDNS
    {
        let mdns_socket = create_multicast_socket().unwrap();
        let announce_packet = mdns_announce_packet();

        task::spawn(async move {
            loop {
                mdns_socket
                    .send_to(&announce_packet, (MULTICAST_ADDR, MULTICAST_PORT))
                    .await
                    .expect("Sending DNS packet failed");

                task::sleep(Duration::from_secs(ANNOUNCEMENT_PER_SEC)).await;
            }
        });
    };

    // Respond to p2p-chat queries and respond with our own IP address via mDNS
    {
        let tx = tx.clone();

        let mdns_socket = create_multicast_socket().unwrap();
        let mut buffer = vec![0; 1024];

        let response_packet = mdns_response_packet(&local_addr);

        task::spawn(async move {
            loop {
                let bytes_read = mdns_socket.recv(&mut buffer).await.unwrap();

                match parse_mdns_packet(&buffer[..bytes_read]) {
                    Some(DiscoveryMessage::Announcement) => {
                        mdns_socket
                            .send_to(&response_packet, (MULTICAST_ADDR, MULTICAST_PORT))
                            .await
                            .expect("Sending DNS packet failed");
                    }
                    Some(DiscoveryMessage::Response(addr)) => {
                        // Make sure to not discover ourselves
                        if addr != local_addr {
                            tx.send(Events::PeerDiscovered(addr))
                                .await
                                .expect("Channel receiver does not exist anymore");
                        }
                    }
                    None => {
                        // Ignore invalid or unrelated DNS packets
                    }
                }
            }
        });
    }

    task::spawn(async move {
        let mut buffer = vec![0; 1024];
        let mut peers: HashSet<SocketAddr> = HashSet::new();

        loop {
            let event = rx.recv().await.unwrap();

            match event {
                Events::PeerDiscovered(peer) => {
                    if peers.insert(peer) {
                        println!("DISCOVERED A NEW PEER {}", peer);
                    }
                }
                Events::SendMessage(message) => {
                    for peer in &peers {
                        // chat_socket.send(message.as_bytes()).await.unwrap();
                    }
                }
            }
        }
    });

    task::spawn(async move {
        let stdin = io::stdin();

        loop {
            let mut line = String::new();
            stdin.read_line(&mut line).await.unwrap();
            tx.send(Events::SendMessage(line)).await.unwrap();
        }
    });

    // Blocking the main thread to not exit before listener and announcer tasks finish (they never
    // will actually)
    let mut signals = Signals::new(&[SIGHUP, SIGTERM, SIGINT, SIGQUIT]).unwrap();
    signals.next().await;
}

fn create_multicast_socket() -> std::io::Result<UdpSocket> {
    let socket = net2::UdpBuilder::new_v4()?
        .reuse_address(true)?
        .reuse_port(true)?
        .bind((Ipv4Addr::UNSPECIFIED, MULTICAST_PORT))?;

    socket.set_multicast_loop_v4(true)?;
    socket.join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED)?;

    Ok(UdpSocket::from(socket))
}

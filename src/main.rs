use std::net::Ipv4Addr;

use net2::unix::UnixUdpBuilderExt;
use simple_dns::{Name, Packet, Question, CLASS, TYPE};

const MULTICAST_PORT: u16 = 5353;
const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

fn main() {
    let mut packet = Packet::new_query(1);

    let question = Question::new(
        Name::new_unchecked("_srv.p2p-chat.local"),
        TYPE::TXT.into(),
        CLASS::IN.into(),
        false,
    );

    packet.questions.push(question);

    let bytes = packet
        .build_bytes_vec()
        .expect("We've created a bad DNS packet");

    let socket = create_socket().unwrap();

    socket.set_multicast_loop_v4(false).unwrap();
    socket
        .join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED)
        .unwrap();

    socket
        .send_to(&bytes, (MULTICAST_ADDR, MULTICAST_PORT))
        .unwrap();
}

fn create_socket() -> std::io::Result<std::net::UdpSocket> {
    net2::UdpBuilder::new_v4()?
        .reuse_address(true)?
        .reuse_port(true)?
        .bind((Ipv4Addr::UNSPECIFIED, MULTICAST_PORT))
}

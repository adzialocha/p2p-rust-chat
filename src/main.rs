use std::net::Ipv4Addr;
use std::time::Duration;

use async_std::net::UdpSocket;
use async_std::task;
use net2::unix::UnixUdpBuilderExt;
use simple_dns::{Name, Packet, Question, CLASS, TYPE};

const MULTICAST_PORT: u16 = 5353;
const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

#[async_std::main]
async fn main() {
    let announcer = {
        let socket = create_socket().unwrap();

        let mut announce_packet = Packet::new_query(1);

        let question = Question::new(
            Name::new_unchecked("_srv.p2p-chat.local"),
            TYPE::TXT.into(),
            CLASS::IN.into(),
            false,
        );

        announce_packet.questions.push(question);

        let bytes = announce_packet
            .build_bytes_vec()
            .expect("We've created a bad DNS packet");

        task::spawn(async move {
            loop {
                socket
                    .send_to(&bytes, (MULTICAST_ADDR, MULTICAST_PORT))
                    .await
                    .expect("Sending DNS packet failed");

                task::sleep(Duration::from_secs(1)).await;
            }
        })
    };

    let listener = task::spawn(async move {
        let socket = create_socket().unwrap();
        let mut buffer = vec![0; 1024];

        loop {
            let (bytes_read, peer) = socket.recv_from(&mut buffer).await.unwrap();
            println!("{}", peer);
            let bytes = &buffer[..bytes_read];

            if let Ok(received_packet) = Packet::parse(bytes) {
                println!("{:?}", received_packet);
            } else {
                println!("Received bad packet");
            }
        }
    });

    announcer.await;
}

fn create_socket() -> std::io::Result<UdpSocket> {
    let socket = net2::UdpBuilder::new_v4()?
        .reuse_address(true)?
        .reuse_port(true)?
        .bind((Ipv4Addr::UNSPECIFIED, MULTICAST_PORT))?;

    socket.set_multicast_loop_v4(true)?;
    socket.join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED)?;

    Ok(UdpSocket::from(socket))
}

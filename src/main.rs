use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use async_std::stream::StreamExt;
use async_std::{channel::unbounded, net::UdpSocket};
use async_std::{io, task};
use net2::unix::UnixUdpBuilderExt;
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook_async_std::Signals;
use simple_dns::{Name, Packet, Question, CLASS, TYPE};

const MULTICAST_PORT: u16 = 5353;
const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

const SERVICE_NAME: &'static str = "_srv.p2p-chat.local";

enum Events {
    PeerDiscovered(SocketAddr),
    SendMessage(String),
}

#[async_std::main]
async fn main() {
    let (tx, rx) = unbounded::<Events>();

    let _announcer = {
        let socket = create_multicast_socket().unwrap();

        let mut announce_packet = Packet::new_query(1);

        let question = Question::new(
            Name::new_unchecked(SERVICE_NAME),
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

    let _listener = {
        let tx = tx.clone();

        task::spawn(async move {
            let socket = create_multicast_socket().unwrap();
            let mut buffer = vec![0; 1024];

            loop {
                let (bytes_read, peer) = socket.recv_from(&mut buffer).await.unwrap();
                let bytes = &buffer[..bytes_read];

                if let Ok(received_packet) = Packet::parse(bytes) {
                    if received_packet.questions.len() == 1
                        && received_packet.questions[0].qname.to_string() == SERVICE_NAME
                    {
                        tx.send(Events::PeerDiscovered(peer))
                            .await
                            .expect("Receiving end does not exist anymore");
                    }
                } else {
                    println!("Received bad packet");
                }
            }
        })
    };

    let _chat = task::spawn(async move {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 9998)).await.unwrap();
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
                        socket.connect(peer).await.unwrap();
                        socket.send(message.as_bytes()).await.unwrap();
                    }
                }
            }

            let (bytes_read, peer) = socket.recv_from(&mut buffer).await.unwrap();
            let bytes = &buffer[..bytes_read];
            println!("{:?} {}", bytes, peer);
        }
    });

    let _std_in = task::spawn(async move {
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

use libp2p::identify::IdentifyEvent;
use libp2p::swarm::{Swarm, SwarmBuilder, SwarmEvent};
use libp2p::Multiaddr;
use libp2p_test::*;
use tokio;

#[tokio::main]
async fn main() {
    env_logger::init();
    let recv_ma: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
    let (recv_pk, transport) = mk_transport();
    let behaviour = MyBehaviour::new(&recv_pk);
    let mut swarm = SwarmBuilder::new(transport, behaviour, recv_pk.into_peer_id())
        .executor(Box::new(Executor(tokio::runtime::Handle::current())))
        .build();
    let sender_ma: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    Swarm::listen_on(&mut swarm, sender_ma).unwrap();
    Swarm::dial_addr(&mut swarm, recv_ma).unwrap();
    let sender = async move {
        // Swarm::dial_addr(&mut swarm, recv_ma.clone()).unwrap();
        // let mut req_id = swarm.ping.send_request(&recv_id, ping.clone());

        loop {
            match swarm.next_event().await {
                SwarmEvent::Behaviour(event) => match event {
                    MyEvent::Identify(id_event) => match id_event {
                        IdentifyEvent::Sent { peer_id } => {
                            println!("Sent identify event to {}", peer_id);
                        }
                        IdentifyEvent::Received { peer_id, .. } => {
                            println!("Received identify event from {}", peer_id)
                        }
                        _ => {}
                    },
                    MyEvent::Rpc(rpc_event) => match rpc_event {
                        RequestResponseEvent::Message { peer, .. } => {
                            println!("Received message from {}", peer);
                        }
                        _ => {}
                    },
                },
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    tokio::time::delay_for(std::time::Duration::from_secs(1)).await;
                    println!("Connected to {}", peer_id);
                    println!("Sending rpc request");
                    let msg = Ping(vec![42; 10]);
                    for _ in 0..5 {
                        swarm.ping.send_request(&peer_id, msg.clone());
                    }
                }
                _ => {}
            }
        }
    };
    sender.await;
}

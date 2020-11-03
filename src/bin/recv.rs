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
    Swarm::listen_on(&mut swarm, recv_ma).unwrap();
    let mut count = 0;
    let recv = async move {
        loop {
            if let SwarmEvent::Behaviour(event) = swarm.next_event().await {
                match event {
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
                        RequestResponseEvent::Message {
                            peer,
                            message:
                                RequestResponseMessage::Request {
                                    request_id,
                                    channel,
                                    ..
                                },
                        } => {
                            println!("Received request id: {} from {}", request_id, peer);
                            println!("Sending response");
                            let msg = Pong(vec![42; 100]);
                            swarm.ping.send_response(channel, msg.clone());
                            count += 1;
                            if count == 5 {
                                return;
                            }
                        }
                        _ => {}
                    },
                }
            }
        }
    };
    recv.await;
}

use libp2p::identify::IdentifyEvent;
use libp2p::swarm::{Swarm, SwarmBuilder, SwarmEvent};
use libp2p::Multiaddr;
use libp2p_test::rpc::methods::*;
use libp2p_test::*;
use rpc::*;
use slog::{o, Drain, Level};
use tokio;

pub fn build_log(level: slog::Level, enabled: bool) -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    if enabled {
        slog::Logger::root(drain.filter_level(level).fuse(), o!())
    } else {
        slog::Logger::root(drain.filter(|_| false).fuse(), o!())
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let log = build_log(Level::Debug, true);
    let recv_ma: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
    let (recv_pk, transport) = mk_transport();
    let behaviour = MyBehaviour::new(&recv_pk, log.clone());
    let mut swarm = SwarmBuilder::new(transport, behaviour, recv_pk.into_peer_id())
        .executor(Box::new(Executor(tokio::runtime::Handle::current())))
        .build();
    Swarm::listen_on(&mut swarm, recv_ma).unwrap();
    let recv = async move {
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
                    MyEvent::Rpc(message) => {
                        let handler_id = message.conn_id;
                        let peer_id = message.peer_id;

                        match message.event {
                            Err(handler_err) => match handler_err {
                                HandlerErr::Inbound { error, .. } => {
                                    println!("Inbound error {}", error);
                                }
                                HandlerErr::Outbound { error, .. } => {
                                    println!("Outbound error {}", error);
                                }
                            },
                            Ok(RPCReceived::Request(id, request)) => {
                                println!("Received request");

                                let peer_request_id = (handler_id, id);
                                match request {
                                    RPCRequest::BlocksByRange(req) => {
                                        let resp = Response::BlocksByRange(Some(vec![42; 42]));
                                        for i in 0..req.count {
                                            swarm.send_successful_response(
                                                peer_id.clone(),
                                                peer_request_id,
                                                resp.clone(),
                                            );
                                            println!("Sent response {}", i);
                                        }

                                        swarm.send_successful_response(
                                            peer_id,
                                            peer_request_id,
                                            Response::BlocksByRange(None),
                                        );
                                        println!("Sent termination");
                                    }
                                }
                            }
                            Ok(RPCReceived::Response(_id, resp)) => match resp {
                                RPCResponse::BlocksByRange(resp) => {
                                    println!("Got response of length {}", resp.len());
                                }
                            },
                            Ok(RPCReceived::EndOfStream(_id, _termination)) => {
                                println!("Got end of stream");
                                return;
                            }
                        }
                    }
                },
                SwarmEvent::ConnectionClosed { .. } => return,
                _ => {}
            }
        }
    };
    recv.await;
}

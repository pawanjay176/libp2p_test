use futures::prelude::*;
use libp2p::core::{
    connection::ConnectionId,
    identity::{self, PublicKey},
    muxing::StreamMuxerBox,
    transport::{self, Transport},
    PeerId,
};
use libp2p::identify::{Identify, IdentifyEvent};
use libp2p::noise::{Keypair, NoiseConfig, X25519Spec};
use libp2p::tcp::TokioTcpConfig;
use libp2p::NetworkBehaviour;
use rpc::*;
use std::pin::Pin;
pub mod rpc;

pub const COUNT: u64 = 256;
// it would usually be MAX_REQUEST_BLOCKS = 1024;

/// Identifier of requests sent by a peer.
pub type PeerRequestId = (ConnectionId, SubstreamId);

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    BlocksByRange(BlocksByRangeRequest),
}

impl std::convert::From<Request> for RPCRequest {
    fn from(req: Request) -> RPCRequest {
        match req {
            Request::BlocksByRange(r) => RPCRequest::BlocksByRange(r),
        }
    }
}

/// The type of RPC responses the Behaviour informs it has received, and allows for sending.
///
// NOTE: This is an application-level wrapper over the lower network level responses that can be
//       sent. The main difference is the absense of Pong and Metadata, which don't leave the
//       Behaviour. For all protocol reponses managed by RPC see `RPCResponse` and
//       `RPCCodedResponse`.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    BlocksByRange(Option<Vec<u8>>),
}

impl std::convert::From<Response> for RPCCodedResponse {
    fn from(resp: Response) -> RPCCodedResponse {
        match resp {
            Response::BlocksByRange(r) => match r {
                Some(b) => RPCCodedResponse::Success(RPCResponse::BlocksByRange(b)),
                None => RPCCodedResponse::StreamTermination(ResponseTermination::BlocksByRange),
            },
        }
    }
}

// use the executor for libp2p
pub struct Executor(pub tokio::runtime::Handle);
impl libp2p::core::Executor for Executor {
    fn exec(&self, f: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.0.spawn(f);
    }
}

pub enum MyEvent {
    Rpc(RPCMessage),
    Identify(IdentifyEvent),
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "MyEvent", event_process = false)]
pub struct MyBehaviour {
    pub rpc: RPC,
    pub identify: Identify,
}

impl From<RPCMessage> for MyEvent {
    fn from(event: RPCMessage) -> Self {
        MyEvent::Rpc(event)
    }
}

impl From<libp2p::identify::IdentifyEvent> for MyEvent {
    fn from(event: libp2p::identify::IdentifyEvent) -> Self {
        MyEvent::Identify(event)
    }
}

impl MyBehaviour {
    pub fn new(local_key: &PublicKey, log: slog::Logger) -> Self {
        let rpc = RPC::new(log.clone());
        let identify = Identify::new("lighthouse/libp2p".into(), "v1".into(), local_key.clone());

        Self { rpc, identify }
    }

    /// Send a request to a peer over RPC.
    pub fn send_request(&mut self, peer_id: PeerId, request_id: RequestId, request: Request) {
        self.rpc.send_request(peer_id, request_id, request.into())
    }

    /// Send a successful response to a peer over RPC.
    pub fn send_successful_response(
        &mut self,
        peer_id: PeerId,
        id: PeerRequestId,
        response: Response,
    ) {
        self.rpc.send_response(peer_id, id, response.into())
    }
}

// use this for libp2p 0.29.0
pub fn mk_transport() -> (PublicKey, transport::Boxed<(PeerId, StreamMuxerBox)>) {
    let transport = TokioTcpConfig::new().nodelay(true);
    let id_keys = identity::Keypair::generate_ed25519();
    let pk = id_keys.public();
    let noise_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&id_keys)
        .unwrap();

    let transport = transport
        .upgrade(libp2p::core::upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(libp2p::mplex::MplexConfig::new())
        .timeout(std::time::Duration::from_secs(10))
        .boxed();
    (pk, transport)
}

// // use this for libp2p 0.28.0
// pub fn mk_transport() -> (
//     PublicKey,
//     transport::boxed::Boxed<(PeerId, StreamMuxerBox), std::io::Error>,
// ) {
//     let transport = TokioTcpConfig::new().nodelay(true);
//     let id_keys = identity::Keypair::generate_ed25519();
//     let pk = id_keys.public();
//     let noise_keys = Keypair::<X25519Spec>::new()
//         .into_authentic(&id_keys)
//         .unwrap();
//
//     let transport = transport
//         .upgrade(libp2p::core::upgrade::Version::V1)
//         .authenticate(NoiseConfig::xx(noise_keys).into_authenticated())
//         .multiplex(libp2p::mplex::MplexConfig::new())
//         .timeout(std::time::Duration::from_secs(10))
//         .map(|(peer, muxer), _| (peer, StreamMuxerBox::new(muxer)))
//         .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
//         .boxed();
//     (pk, transport)
// }

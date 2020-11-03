use async_trait::async_trait;
use futures::prelude::*;
use libp2p::core::{
    identity::{self, PublicKey},
    muxing::StreamMuxerBox,
    transport::{self, Transport},
    upgrade::{self, read_one, write_one},
    PeerId,
};
use libp2p::identify::{Identify, IdentifyEvent};
use libp2p::noise::{Keypair, NoiseConfig, X25519Spec};
pub use libp2p::request_response::*;
use libp2p::swarm::NetworkBehaviourEventProcess;
use libp2p::tcp::TokioTcpConfig;
use libp2p::NetworkBehaviour;
use std::pin::Pin;
use std::{io, iter};

// use the executor for libp2p
pub struct Executor(pub tokio::runtime::Handle);
impl libp2p::core::Executor for Executor {
    fn exec(&self, f: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.0.spawn(f);
    }
}
#[derive(Debug, Clone)]
pub struct PingProtocol();
#[derive(Clone)]
pub struct PingCodec();
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ping(pub Vec<u8>);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pong(pub Vec<u8>);

impl ProtocolName for PingProtocol {
    fn protocol_name(&self) -> &[u8] {
        "/ping/1".as_bytes()
    }
}

pub enum MyEvent {
    Rpc(RequestResponseEvent<Ping, Pong>),
    Identify(IdentifyEvent),
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "MyEvent", event_process = false)]
pub struct MyBehaviour {
    pub ping: RequestResponse<PingCodec>,
    pub identify: Identify,
}

impl From<RequestResponseEvent<Ping, Pong>> for MyEvent {
    fn from(event: RequestResponseEvent<Ping, Pong>) -> Self {
        MyEvent::Rpc(event)
    }
}

impl From<libp2p::identify::IdentifyEvent> for MyEvent {
    fn from(event: libp2p::identify::IdentifyEvent) -> Self {
        MyEvent::Identify(event)
    }
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<Ping, Pong>> for MyBehaviour {
    // Called when `floodsub` produces an event.
    fn inject_event(&mut self, _message: RequestResponseEvent<Ping, Pong>) {
        println!("Got event");
    }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for MyBehaviour {
    // Called when `mdns` produces an event.
    fn inject_event(&mut self, _event: IdentifyEvent) {
        println!("Got id event");
    }
}

impl MyBehaviour {
    pub fn new(local_key: &PublicKey) -> Self {
        let protocols = iter::once((PingProtocol(), ProtocolSupport::Full));
        let cfg = RequestResponseConfig::default();
        let ping = RequestResponse::new(PingCodec(), protocols, cfg);

        let identify = Identify::new("lighthouse/libp2p".into(), "v1".into(), local_key.clone());

        Self { ping, identify }
    }
}

#[async_trait]
impl RequestResponseCodec for PingCodec {
    type Protocol = PingProtocol;
    type Request = Ping;
    type Response = Pong;

    async fn read_request<T>(&mut self, _: &PingProtocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_one(io, 1024)
            .map(|res| match res {
                Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                Ok(vec) if vec.is_empty() => Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(vec) => Ok(Ping(vec)),
            })
            .await
    }

    async fn read_response<T>(&mut self, _: &PingProtocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_one(io, 1024)
            .map(|res| match res {
                Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                Ok(vec) if vec.is_empty() => Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(vec) => Ok(Pong(vec)),
            })
            .await
    }

    async fn write_request<T>(
        &mut self,
        _: &PingProtocol,
        io: &mut T,
        Ping(data): Ping,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_one(io, data).await
    }

    async fn write_response<T>(
        &mut self,
        _: &PingProtocol,
        io: &mut T,
        Pong(data): Pong,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_one(io, data).await
    }
}

pub fn mk_transport() -> (PublicKey, transport::Boxed<(PeerId, StreamMuxerBox)>) {
    let id_keys = identity::Keypair::generate_ed25519();
    let pk = id_keys.public();
    let noise_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&id_keys)
        .unwrap();
    (
        pk,
        TokioTcpConfig::new()
            .nodelay(true)
            .upgrade(upgrade::Version::V1)
            .authenticate(NoiseConfig::xx(noise_keys).into_authenticated())
            .multiplex(libp2p::mplex::MplexConfig::new())
            .boxed(),
    )
}

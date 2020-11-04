use crate::rpc::methods::*;
use crate::rpc::protocol::{Protocol, ProtocolId, RPCError, Version};
use crate::rpc::{RPCCodedResponse, RPCRequest, RPCResponse};
use libp2p::bytes::{BufMut, Bytes, BytesMut};
use ssz::{Decode, Encode};
use tokio_util::codec::{Decoder, Encoder};
use unsigned_varint::codec::UviBytes;

/* Inbound Codec */

pub struct SimpleInboundCodec {
    protocol: ProtocolId,
    inner: UviBytes,
}

impl SimpleInboundCodec {
    pub fn new(protocol: ProtocolId) -> Self {
        let uvi_codec = UviBytes::default();

        SimpleInboundCodec {
            inner: uvi_codec,
            protocol,
        }
    }
}

// Encoder for inbound streams: Encodes RPC Responses sent to peers.
impl Encoder<RPCCodedResponse> for SimpleInboundCodec {
    type Error = RPCError;

    fn encode(&mut self, item: RPCCodedResponse, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = match item {
            RPCCodedResponse::Success(resp) => match resp {
                RPCResponse::BlocksByRange(res) => res,
            },
            RPCCodedResponse::Error(_, err) => err.into_bytes(),
            RPCCodedResponse::StreamTermination(_) => {
                unreachable!("Code error - attempting to encode a stream termination")
            }
        };
        if !bytes.is_empty() {
            return self
                .inner
                .encode(Bytes::from(bytes), dst)
                .map_err(RPCError::from);
        } else {
            dst.reserve(1);
            dst.put_u8(0);
        }

        Ok(())
    }
}

// Decoder for inbound streams: Decodes RPC requests from peers
impl Decoder for SimpleInboundCodec {
    type Item = RPCRequest;
    type Error = RPCError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.inner.decode(src).map_err(RPCError::from) {
            Ok(Some(decoded_buffer)) => match self.protocol.message_name {
                Protocol::BlocksByRange => match self.protocol.version {
                    Version::V1 => Ok(Some(RPCRequest::BlocksByRange(
                        BlocksByRangeRequest::from_ssz_bytes(&decoded_buffer)?,
                    ))),
                },
            },
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/* Outbound Codec: Codec for initiating RPC requests */
pub struct SimpleOutboundCodec {
    inner: UviBytes,
    protocol: ProtocolId,
}

impl SimpleOutboundCodec {
    pub fn new(protocol: ProtocolId) -> Self {
        let uvi_codec = UviBytes::default();

        SimpleOutboundCodec {
            inner: uvi_codec,
            protocol,
        }
    }
}

// Encoder for outbound streams: Encodes RPC Requests to peers
impl Encoder<RPCRequest> for SimpleOutboundCodec {
    type Error = RPCError;

    fn encode(&mut self, item: RPCRequest, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = match item {
            RPCRequest::BlocksByRange(req) => req.as_ssz_bytes(),
        };
        // length-prefix
        self.inner
            .encode(Bytes::from(bytes), dst)
            .map_err(RPCError::from)
    }
}

impl Decoder for SimpleOutboundCodec {
    type Item = RPCCodedResponse;
    type Error = RPCError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() == 1 && src[0] == 0_u8 {
            src.clear();
            Err(RPCError::InvalidData)
        } else {
            match self.inner.decode(src).map_err(RPCError::from) {
                Ok(Some(decoded_buffer)) => match self.protocol.message_name {
                    Protocol::BlocksByRange => match self.protocol.version {
                        Version::V1 => Ok(Some(RPCCodedResponse::Success(
                            RPCResponse::BlocksByRange(decoded_buffer.to_vec()),
                        ))),
                    },
                },
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
}

use crate::rpc::methods::*;
use crate::rpc::{
    codec::base::OutboundCodec,
    protocol::{Encoding, Protocol, ProtocolId, RPCError, Version},
};
use crate::rpc::{RPCCodedResponse, RPCRequest, RPCResponse};
use libp2p::bytes::{BufMut, Bytes, BytesMut};
use ssz::{Decode, Encode};
use tokio_util::codec::{Decoder, Encoder};
use unsigned_varint::codec::UviBytes;

/* Inbound Codec */

pub struct SSZSnappyInboundCodec {
    protocol: ProtocolId,
    inner: UviBytes,
}

impl SSZSnappyInboundCodec {
    pub fn new(protocol: ProtocolId, max_packet_size: usize) -> Self {
        let mut uvi_codec = UviBytes::default();
        uvi_codec.set_max_len(max_packet_size);
        // this encoding only applies to ssz_snappy.
        debug_assert_eq!(protocol.encoding, Encoding::SSZSnappy);

        SSZSnappyInboundCodec {
            inner: uvi_codec,
            protocol,
        }
    }
}

// Encoder for inbound streams: Encodes RPC Responses sent to peers.
impl Encoder<RPCCodedResponse> for SSZSnappyInboundCodec {
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
        // // SSZ encoded bytes should be within `max_packet_size`
        // if bytes.len() > self.max_packet_size {
        //     return Err(RPCError::InternalError(
        //         "attempting to encode data > max_packet_size",
        //     ));
        // }
        // Inserts the length prefix of the uncompressed bytes into dst
        // encoded as a unsigned varint
        if !bytes.is_empty() {
            return self
                .inner
                .encode(Bytes::from(bytes), dst)
                .map_err(RPCError::from);
        } else {
            dst.reserve(1);
            dst.put_u8(0);
        }

        // let mut writer = FrameEncoder::new(Vec::new());
        // writer.write_all(&bytes).map_err(RPCError::from)?;
        // writer.flush().map_err(RPCError::from)?;

        // // Write compressed bytes to `dst`
        // dst.extend_from_slice(writer.get_ref());
        Ok(())
    }
}

// Decoder for inbound streams: Decodes RPC requests from peers
impl Decoder for SSZSnappyInboundCodec {
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
pub struct SSZSnappyOutboundCodec {
    inner: UviBytes,
    protocol: ProtocolId,
}

impl SSZSnappyOutboundCodec {
    pub fn new(protocol: ProtocolId, max_packet_size: usize) -> Self {
        let mut uvi_codec = UviBytes::default();
        uvi_codec.set_max_len(max_packet_size);
        // this encoding only applies to ssz_snappy.
        debug_assert_eq!(protocol.encoding, Encoding::SSZSnappy);

        SSZSnappyOutboundCodec {
            inner: uvi_codec,
            protocol,
        }
    }
}

// Encoder for outbound streams: Encodes RPC Requests to peers
impl Encoder<RPCRequest> for SSZSnappyOutboundCodec {
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

// Decoder for outbound streams: Decodes RPC responses from peers.
//
// The majority of the decoding has now been pushed upstream due to the changing specification.
// We prefer to decode blocks and attestations with extra knowledge about the chain to perform
// faster verification checks before decoding entire blocks/attestations.
impl Decoder for SSZSnappyOutboundCodec {
    type Item = RPCResponse;
    type Error = RPCError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() == 1 && src[0] == 0_u8 {
            src.clear();
            Err(RPCError::InvalidData)
        } else {
            match self.inner.decode(src).map_err(RPCError::from) {
                Ok(Some(decoded_buffer)) => match self.protocol.message_name {
                    Protocol::BlocksByRange => match self.protocol.version {
                        Version::V1 => {
                            Ok(Some(RPCResponse::BlocksByRange(decoded_buffer.to_vec())))
                        }
                    },
                },
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
}

impl OutboundCodec<RPCRequest> for SSZSnappyOutboundCodec {
    type CodecErrorType = ErrorType;

    fn decode_error(
        &mut self,
        src: &mut BytesMut,
    ) -> Result<Option<Self::CodecErrorType>, RPCError> {
        match self.inner.decode(src).map_err(RPCError::from) {
            Ok(Some(decoded_buffer)) => Ok(Some(
                String::from_utf8(decoded_buffer.to_vec())
                    .map_err(|_| RPCError::IoError("decoding error".to_string()))?,
            )),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// / Handle errors that we get from decoding an RPC message from the stream.
// / `num_bytes_read` is the number of bytes the snappy decoder has read from the underlying stream.
// / `max_compressed_len` is the maximum compressed size for a given uncompressed size.
// fn handle_error<T>(
//     err: std::io::Error,
//     num_bytes: u64,
//     max_compressed_len: u64,
// ) -> Result<Option<T>, RPCError> {
//     match err.kind() {
//         ErrorKind::UnexpectedEof => {
//             // If snappy has read `max_compressed_len` from underlying stream and still can't fill buffer, we have a malicious message.
//             // Report as `InvalidData` so that malicious peer gets banned.
//             if num_bytes >= max_compressed_len {
//                 Err(RPCError::InvalidData)
//             } else {
//                 // Haven't received enough bytes to decode yet, wait for more
//                 Ok(None)
//             }
//         }
//         _ => Err(err).map_err(RPCError::from),
//     }
// }

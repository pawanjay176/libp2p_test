use crate::rpc::methods::*;
use crate::rpc::{
    codec::base::OutboundCodec,
    protocol::{Encoding, Protocol, ProtocolId, RPCError, Version},
};
use crate::rpc::{RPCCodedResponse, RPCRequest, RPCResponse};
use libp2p::bytes::BytesMut;
use snap::read::FrameDecoder;
use snap::write::FrameEncoder;
use ssz::{Decode, Encode};
use std::io::Cursor;
use std::io::ErrorKind;
use std::io::{Read, Write};
use tokio_util::codec::{Decoder, Encoder};
use unsigned_varint::codec::Uvi;

/* Inbound Codec */

pub struct SSZSnappyInboundCodec {
    protocol: ProtocolId,
    inner: Uvi<usize>,
    len: Option<usize>,
    /// Maximum bytes that can be sent in one req/resp chunked responses.
    max_packet_size: usize,
}

impl SSZSnappyInboundCodec {
    pub fn new(protocol: ProtocolId, max_packet_size: usize) -> Self {
        let uvi_codec = Uvi::default();
        // this encoding only applies to ssz_snappy.
        debug_assert_eq!(protocol.encoding, Encoding::SSZSnappy);

        SSZSnappyInboundCodec {
            inner: uvi_codec,
            protocol,
            len: None,
            max_packet_size,
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
        // SSZ encoded bytes should be within `max_packet_size`
        if bytes.len() > self.max_packet_size {
            return Err(RPCError::InternalError(
                "attempting to encode data > max_packet_size",
            ));
        }
        // Inserts the length prefix of the uncompressed bytes into dst
        // encoded as a unsigned varint
        self.inner
            .encode(bytes.len(), dst)
            .map_err(RPCError::from)?;

        let mut writer = FrameEncoder::new(Vec::new());
        writer.write_all(&bytes).map_err(RPCError::from)?;
        writer.flush().map_err(RPCError::from)?;

        // Write compressed bytes to `dst`
        dst.extend_from_slice(writer.get_ref());
        Ok(())
    }
}

// Decoder for inbound streams: Decodes RPC requests from peers
impl Decoder for SSZSnappyInboundCodec {
    type Item = RPCRequest;
    type Error = RPCError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.len.is_none() {
            // Decode the length of the uncompressed bytes from an unsigned varint
            // Note: length-prefix of > 10 bytes(uint64) would be a decoding error
            match self.inner.decode(src).map_err(RPCError::from)? {
                Some(length) => {
                    self.len = Some(length);
                }
                None => return Ok(None), // need more bytes to decode length
            }
        };

        let length = self.len.expect("length should be Some");

        // Should not attempt to decode rpc chunks with `length > max_packet_size` or not within bounds of
        // packet size for ssz container corresponding to `self.protocol`.
        let ssz_limits = self.protocol.rpc_request_limits();
        if length > self.max_packet_size || ssz_limits.is_out_of_bounds(length) {
            return Err(RPCError::InvalidData);
        }
        // Calculate worst case compression length for given uncompressed length
        let max_compressed_len = snap::raw::max_compress_len(length) as u64;

        // Create a limit reader as a wrapper that reads only upto `max_compressed_len` from `src`.
        let limit_reader = Cursor::new(src.as_ref()).take(max_compressed_len);
        let mut reader = FrameDecoder::new(limit_reader);
        let mut decoded_buffer = vec![0; length];

        match reader.read_exact(&mut decoded_buffer) {
            Ok(()) => {
                // `n` is how many bytes the reader read in the compressed stream
                let n = reader.get_ref().get_ref().position();
                self.len = None;
                let _read_bytes = src.split_to(n as usize);

                // We need not check that decoded_buffer.len() is within bounds here
                // since we have already checked `length` above.
                match self.protocol.message_name {
                    Protocol::BlocksByRange => match self.protocol.version {
                        Version::V1 => Ok(Some(RPCRequest::BlocksByRange(
                            BlocksByRangeRequest::from_ssz_bytes(&decoded_buffer)?,
                        ))),
                    },
                }
            }
            Err(e) => handle_error(e, reader.get_ref().get_ref().position(), max_compressed_len),
        }
    }
}

/* Outbound Codec: Codec for initiating RPC requests */
pub struct SSZSnappyOutboundCodec {
    inner: Uvi<usize>,
    len: Option<usize>,
    protocol: ProtocolId,
    /// Maximum bytes that can be sent in one req/resp chunked responses.
    max_packet_size: usize,
}

impl SSZSnappyOutboundCodec {
    pub fn new(protocol: ProtocolId, max_packet_size: usize) -> Self {
        let uvi_codec = Uvi::default();
        // this encoding only applies to ssz_snappy.
        debug_assert_eq!(protocol.encoding, Encoding::SSZSnappy);

        SSZSnappyOutboundCodec {
            inner: uvi_codec,
            protocol,
            max_packet_size,
            len: None,
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
        // SSZ encoded bytes should be within `max_packet_size`
        if bytes.len() > self.max_packet_size {
            return Err(RPCError::InternalError(
                "attempting to encode data > max_packet_size",
            ));
        }

        // Inserts the length prefix of the uncompressed bytes into dst
        // encoded as a unsigned varint
        self.inner
            .encode(bytes.len(), dst)
            .map_err(RPCError::from)?;

        let mut writer = FrameEncoder::new(Vec::new());
        writer.write_all(&bytes).map_err(RPCError::from)?;
        writer.flush().map_err(RPCError::from)?;

        // Write compressed bytes to `dst`
        dst.extend_from_slice(writer.get_ref());
        Ok(())
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
        if self.len.is_none() {
            // Decode the length of the uncompressed bytes from an unsigned varint
            // Note: length-prefix of > 10 bytes(uint64) would be a decoding error
            match self.inner.decode(src).map_err(RPCError::from)? {
                Some(length) => {
                    self.len = Some(length as usize);
                }
                None => return Ok(None), // need more bytes to decode length
            }
        };

        let length = self.len.expect("length should be Some");

        // Should not attempt to decode rpc chunks with `length > max_packet_size` or not within bounds of
        // packet size for ssz container corresponding to `self.protocol`.
        let ssz_limits = self.protocol.rpc_response_limits();
        if length > self.max_packet_size || ssz_limits.is_out_of_bounds(length) {
            return Err(RPCError::InvalidData);
        }
        // Calculate worst case compression length for given uncompressed length
        let max_compressed_len = snap::raw::max_compress_len(length) as u64;
        // Create a limit reader as a wrapper that reads only upto `max_compressed_len` from `src`.
        let limit_reader = Cursor::new(src.as_ref()).take(max_compressed_len);
        let mut reader = FrameDecoder::new(limit_reader);

        let mut decoded_buffer = vec![0; length];

        match reader.read_exact(&mut decoded_buffer) {
            Ok(()) => {
                // `n` is how many bytes the reader read in the compressed stream
                let n = reader.get_ref().get_ref().position();
                self.len = None;
                let _read_bytes = src.split_to(n as usize);

                // We need not check that decoded_buffer.len() is within bounds here
                // since we have already checked `length` above.
                match self.protocol.message_name {
                    Protocol::BlocksByRange => match self.protocol.version {
                        Version::V1 => Ok(Some(RPCResponse::BlocksByRange(decoded_buffer))),
                    },
                }
            }
            Err(e) => handle_error(e, reader.get_ref().get_ref().position(), max_compressed_len),
        }
    }
}

impl OutboundCodec<RPCRequest> for SSZSnappyOutboundCodec {
    type CodecErrorType = ErrorType;

    fn decode_error(
        &mut self,
        src: &mut BytesMut,
    ) -> Result<Option<Self::CodecErrorType>, RPCError> {
        if self.len.is_none() {
            // Decode the length of the uncompressed bytes from an unsigned varint
            match self.inner.decode(src).map_err(RPCError::from)? {
                Some(length) => {
                    self.len = Some(length as usize);
                }
                None => return Ok(None), // need more bytes to decode length
            }
        };

        let length = self.len.expect("length should be Some");

        // Should not attempt to decode rpc chunks with `length > max_packet_size` or not within bounds of
        // packet size for ssz container corresponding to `ErrorType`.
        if length > self.max_packet_size || length > 10000 || length < 1 {
            return Err(RPCError::InvalidData);
        }

        // Calculate worst case compression length for given uncompressed length
        let max_compressed_len = snap::raw::max_compress_len(length) as u64;
        // // Create a limit reader as a wrapper that reads only upto `max_compressed_len` from `src`.
        let limit_reader = Cursor::new(src.as_ref()).take(max_compressed_len);
        let mut reader = FrameDecoder::new(limit_reader);
        let mut decoded_buffer = vec![0; length];
        match reader.read_exact(&mut decoded_buffer) {
            Ok(()) => {
                // `n` is how many bytes the reader read in the compressed stream
                let n = reader.get_ref().get_ref().position();
                self.len = None;
                let _read_bytes = src.split_to(n as usize);
                Ok(Some(String::from_utf8(decoded_buffer).map_err(|_| {
                    RPCError::IoError("failed to decode stuff".to_string())
                })?))
            }
            Err(e) => handle_error(e, reader.get_ref().get_ref().position(), max_compressed_len),
        }
    }
}

/// Handle errors that we get from decoding an RPC message from the stream.
/// `num_bytes_read` is the number of bytes the snappy decoder has read from the underlying stream.
/// `max_compressed_len` is the maximum compressed size for a given uncompressed size.
fn handle_error<T>(
    err: std::io::Error,
    num_bytes: u64,
    max_compressed_len: u64,
) -> Result<Option<T>, RPCError> {
    match err.kind() {
        ErrorKind::UnexpectedEof => {
            // If snappy has read `max_compressed_len` from underlying stream and still can't fill buffer, we have a malicious message.
            // Report as `InvalidData` so that malicious peer gets banned.
            if num_bytes >= max_compressed_len {
                Err(RPCError::InvalidData)
            } else {
                // Haven't received enough bytes to decode yet, wait for more
                Ok(None)
            }
        }
        _ => Err(err).map_err(RPCError::from),
    }
}

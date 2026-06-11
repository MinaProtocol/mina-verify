//! Mina libp2p RPC (`coda/rpcs/0.0.1`) — the deterministic request/response path
//! (no gossip-mesh wait). This module implements the **wire protocol**:
//! frame/handshake/query and reading the `get_best_tip` response into a block, over
//! any already-open RPC stream ([`rpc_best_tip`]).
//!
//! Wire protocol (reverse-engineered from openmina's rpc_kernel):
//!   message = [8-byte LE length][binprot MessageHeader][payload]
//!   on open: Handshake = a Response with id = b"RPC\0\0\0\0\0", payload 0x01
//!   query:   MessageHeader::Query(QueryHeader{tag:"get_best_tip", version:2, id}) + NeedsLength(())
//!   reply:   MessageHeader::Response{id} + RpcResult<NeedsLength<GetBestTipV2Response>, Error>
//!
//! Transport: [`crate::rpc_net`] opens the `coda/rpcs/0.0.1` substream (a custom libp2p
//! `ConnectionHandler` over the fork's libp2p) and hands the stream to [`rpc_best_tip`].
//! (crates.io `libp2p-stream` can't be used — the fork is a monorepo and its swarm
//! version can't be `[patch]`-unified with the external crate.)

use binprot::{BinProtRead, BinProtWrite};
use libp2p::futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use mina_p2p_messages::rpc::GetBestTipV2;
use mina_p2p_messages::rpc_kernel::{
    MessageHeader, NeedsLength, QueryHeader, QueryPayload, ResponseHeader, ResponsePayload,
    RpcMethod,
};
use mina_p2p_messages::v2::MinaBlockBlockStableV2;

const HANDSHAKE_ID: u64 = u64::from_le_bytes(*b"RPC\x00\x00\x00\x00\x00");
const QUERY_ID: u64 = 1;

/// `coda/rpcs/0.0.1` (no leading `/` — the Mina convention the fork's libp2p allows).
pub const RPC_PROTOCOL: &str = "coda/rpcs/0.0.1";

/// `[8-byte LE length][header][payload]` framing.
fn frame(header: &MessageHeader, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 8];
    header.binprot_write(&mut v).expect("write header");
    v.extend_from_slice(payload);
    let len = ((v.len() - 8) as u64).to_le_bytes();
    v[..8].copy_from_slice(&len);
    v
}

/// The RPC handshake sent on stream open.
pub fn handshake_bytes() -> Vec<u8> {
    frame(&MessageHeader::Response(ResponseHeader { id: HANDSHAKE_ID }), b"\x01")
}

/// A framed `get_best_tip` query.
pub fn best_tip_query_bytes() -> Vec<u8> {
    let mut payload = Vec::new();
    QueryPayload::<<GetBestTipV2 as RpcMethod>::Query>::binprot_write(&NeedsLength(()), &mut payload)
        .expect("write query payload");
    let header = MessageHeader::Query(QueryHeader {
        tag: GetBestTipV2::NAME.into(),
        version: GetBestTipV2::VERSION,
        id: QUERY_ID,
    });
    frame(&header, &payload)
}

/// Run `get_best_tip` over an already-open `coda/rpcs/0.0.1` stream: send the
/// handshake + query, read messages (skipping the peer's handshake/heartbeats) until
/// the response, and return the tip block (which the caller verifies with mina_verify).
pub async fn rpc_best_tip<S>(mut stream: S) -> Result<MinaBlockBlockStableV2, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    stream.write_all(&handshake_bytes()).await.map_err(|e| e.to_string())?;
    stream.write_all(&best_tip_query_bytes()).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;

    loop {
        let mut len_buf = [0u8; 8];
        stream.read_exact(&mut len_buf).await.map_err(|e| e.to_string())?;
        let len = u64::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.map_err(|e| e.to_string())?;

        let mut cursor = &buf[..];
        let header = MessageHeader::binprot_read(&mut cursor).map_err(|e| e.to_string())?;
        match header {
            MessageHeader::Response(ResponseHeader { id }) if id == QUERY_ID => {
                let payload: ResponsePayload<<GetBestTipV2 as RpcMethod>::Response> =
                    BinProtRead::binprot_read(&mut cursor).map_err(|e| e.to_string())?;
                let best_tip = payload
                    .0
                    .map_err(|_| "rpc error response".to_string())?
                    .0 // NeedsLength -> inner
                    .ok_or_else(|| "peer has no best tip".to_string())?;
                return Ok(best_tip.data);
            }
            // peer handshake, heartbeats, or other responses — keep reading.
            _ => continue,
        }
    }
}

//! A pending transaction is replacement authority only after hash and signature verification.
use super::*;
use crate::chains::evm::tx::{DecodedUnsigned, decode_pending_original};

pub async fn pending_original(
    p: &EvmProvider,
    net: &Net,
    hash: B256,
    from: Address,
    nonce: u64,
) -> Result<Option<DecodedUnsigned>> {
    if p.net.chain.key != net.chain.key {
        return Err(WalletError::validation(
            "original transaction provider is on another chain",
        ));
    }
    let call = p
        .client()
        .request::<_, Option<Value>>("eth_getTransactionByHash", (hash,));
    let Some(value) = p.rpc("eth_getTransactionByHash", call).await? else {
        return Ok(None);
    };
    decode_pending_original(&value, config(net.chain).chain_id, hash, from, nonce).map(Some)
}

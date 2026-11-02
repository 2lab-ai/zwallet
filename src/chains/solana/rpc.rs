//! Bounded, redacted JSON-RPC. Genesis responses identify a service; they are not consensus proofs.
use super::config;
use crate::chains::{Net, url_origin};
use crate::core::error::{Result, WalletError};
use alloy::transports::http::reqwest::{self, Url};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_REPLY: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(15);
pub struct Client {
    http: reqwest::Client,
    net: Net,
    url: Url,
}
impl Client {
    fn new(net: &Net) -> Result<Self> {
        let url: Url = net
            .rpc_url
            .parse()
            .map_err(|_| WalletError::validation("invalid Solana RPC URL"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(WalletError::validation(
                "Solana RPC URL must use HTTP or HTTPS",
            ));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .user_agent(concat!("zwallet/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| WalletError::rpc("cannot start Solana HTTP client"))?;
        Ok(Self {
            http,
            net: net.clone(),
            url,
        })
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(TIMEOUT, self.exchange(method, params))
            .await
            .map_err(|_| WalletError::rpc("Solana RPC request timed out"))?
    }
    async fn exchange(&self, method: &str, params: Value) -> Result<Value> {
        // Never echo the body/error text: a server can reflect credentials or signed wire bytes.
        let mut reply = self
            .http
            .post(self.url.clone())
            .header("content-type", "application/json")
            .body(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string())
            .send()
            .await
            .map_err(|_| {
                WalletError::rpc(format!(
                    "Solana RPC transport failed at {}",
                    url_origin(&self.net.rpc_url)
                ))
            })?;
        if !reply.status().is_success() {
            return Err(WalletError::rpc(format!(
                "Solana RPC HTTP {} at {}",
                reply.status().as_u16(),
                url_origin(&self.net.rpc_url)
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reply
            .chunk()
            .await
            .map_err(|_| WalletError::rpc("cannot read Solana RPC response"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_REPLY {
                return Err(WalletError::rpc("Solana RPC response exceeds 1 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| WalletError::rpc("Solana RPC response is not valid JSON"))?;
        if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || value.get("id") != Some(&json!(1))
        {
            return Err(WalletError::rpc(
                "Solana RPC response has a wrong JSON-RPC envelope",
            ));
        }
        if let Some(e) = value.get("error") {
            return Err(
                WalletError::rpc("Solana RPC rejected the request").with_detail(
                    "rpcCode",
                    e.get("code")
                        .and_then(Value::as_i64)
                        .map_or(Value::Null, Value::from),
                ),
            );
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| WalletError::rpc("Solana RPC response has no result"))
    }
}
pub async fn connect(net: &Net) -> Result<Client> {
    let client = Client::new(net)?;
    let want = config(net.chain)?.genesis;
    if client.call("getGenesisHash", json!([])).await?.as_str() != Some(want) {
        return Err(WalletError::rpc(
            "Solana RPC genesis hash does not match the selected network",
        ));
    }
    Ok(client)
}

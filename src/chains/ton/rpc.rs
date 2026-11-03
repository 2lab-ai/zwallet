//! TON Center compatible v2 JSON-RPC and same-origin v3 indexed receipt reads. Replies are
//! bounded, untrusted observations; the zerostate check is not a light-client proof.
use super::{address, cell, config, tx};
use crate::{
    chains::Net,
    core::{
        error::{Result, WalletError},
        text::{PROSE_MAX, ascii_inert},
    },
};
use alloy::transports::http::reqwest::{self, Url};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) const ZERO_ROOT: &str = "F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=";
pub(super) const ZERO_FILE: &str = "XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24=";
const LIMIT: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(15);
pub(super) struct Client {
    http: reqwest::Client,
    url: Url,
    net: Net,
    paced: bool,
}
impl Client {
    fn new(net: &Net) -> Result<Self> {
        let url = Url::parse(&net.rpc_url).map_err(|_| WalletError::rpc("invalid TON RPC URL"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(WalletError::rpc("TON RPC URL must use HTTP or HTTPS"));
        }
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| WalletError::rpc("cannot create TON RPC client"))?;
        // Proactively space default-provider requests, including the first sendBoc. Read-only
        // retries alone would still burst a send immediately after its seqno preflight.
        let paced = url.host_str() == Some("toncenter.com");
        Ok(Self {
            http,
            url,
            net: net.clone(),
            paced,
        })
    }
    fn error(&self, s: &str) -> WalletError {
        WalletError::rpc(ascii_inert(&self.net.redact(s), PROSE_MAX))
    }
    async fn pace(&self) {
        if !self.paced {
            return;
        }
        static LAST_REQUEST: std::sync::OnceLock<tokio::sync::Mutex<Option<tokio::time::Instant>>> =
            std::sync::OnceLock::new();
        let mut last = LAST_REQUEST
            .get_or_init(|| tokio::sync::Mutex::new(None))
            .lock()
            .await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + Duration::from_millis(1100)).await;
        }
        *last = Some(tokio::time::Instant::now());
    }
    #[cfg(test)]
    pub(super) fn enable_test_pacing(&mut self) {
        self.paced = true;
    }
    async fn read(&self, req: reqwest::RequestBuilder, retry_read: bool) -> Result<Value> {
        // TON Center's keyless allowance is one request per second. Retry only a positively
        // identified rate-limit rejection of a read, within one overall deadline. A sendBoc
        // request is never replayed after any response/timeout/transport failure.
        tokio::time::timeout(Duration::from_secs(20), async {
            for attempt in 0..=2 {
                let request = req
                    .try_clone()
                    .ok_or_else(|| self.error("cannot copy TON RPC read"))?;
                self.pace().await;
                let mut response = request
                    .send()
                    .await
                    .map_err(|e| self.error(&e.to_string()))?;
                let status = response.status();
                if status.as_u16() == 429 && retry_read && attempt < 2 {
                    drop(response);
                    tokio::time::sleep(Duration::from_millis(1100)).await;
                    continue;
                }
                let mut b = Vec::new();
                while let Some(c) = response
                    .chunk()
                    .await
                    .map_err(|e| self.error(&e.to_string()))?
                {
                    if b.len() + c.len() > LIMIT {
                        return Err(self.error("TON RPC response exceeds 1 MiB"));
                    }
                    b.extend(c);
                }
                if !status.is_success() {
                    return Err(self.error(&format!("TON RPC HTTP status {}", status.as_u16())));
                }
                return serde_json::from_slice(&b)
                    .map_err(|_| self.error("TON RPC returned invalid JSON"));
            }
            Err(self.error("TON RPC rate-limit retry exhausted"))
        })
        .await
        .map_err(|_| self.error("TON RPC operation exceeded its 20-second deadline"))?
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let v = self
            .read(
                self.http
                    .post(self.url.clone())
                    .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})),
                method != "sendBoc",
            )
            .await?;
        if v.get("error").is_some() || v.get("ok") == Some(&json!(false)) {
            return Err(self.error(&format!("TON RPC {method} refused the request")));
        }
        if v.get("id").is_some_and(|x| x != &json!(1)) {
            return Err(self.error("TON RPC response id mismatch"));
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| self.error("TON RPC response has no result"))
    }
    pub async fn by_body_hash(&self, hash: &str) -> Result<Value> {
        let mut url = self.url.clone();
        let path = url.path().to_string();
        let prefix=path.rsplit_once("/v2/").map(|(p,_)|p).or_else(||path.strip_suffix("/v2")).ok_or_else(||self.error("TON receipt lookup needs a TON Center compatible /v2/ RPC and same-origin /v3/ index"))?;
        url.set_path(&format!("{prefix}/v3/transactionsByMessage"));
        url.query_pairs_mut()
            .append_pair("body_hash", hash)
            .append_pair("direction", "in")
            .append_pair("limit", "2");
        self.read(self.http.get(url), true).await
    }
}
pub(super) async fn connect(net: &Net) -> Result<Client> {
    if config(net.chain)?.global_id != -239 {
        return Err(WalletError::rpc("only TON mainnet is supported"));
    }
    let c = Client::new(net)?;
    let v = c.call("getMasterchainInfo", json!({})).await?;
    if v["init"]["root_hash"] != ZERO_ROOT
        || v["init"]["file_hash"] != ZERO_FILE
        || v["init"]["seqno"] != 0
        || v["init"]["workchain"] != -1
    {
        return Err(WalletError::rpc(
            "TON RPC zerostate does not match mainnet; refusing the endpoint",
        ));
    }
    Ok(c)
}
pub(super) struct Account {
    pub balance: u64,
    pub seqno: u32,
    pub public: Option<[u8; 32]>,
    pub active: bool,
}
pub(super) fn number(v: &Value) -> Result<u64> {
    v.as_u64()
        .or_else(|| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse().ok())
        })
        .ok_or_else(|| WalletError::rpc("TON RPC amount is not an unsigned 64-bit integer"))
}
pub(super) async fn account(c: &Client, owner: &str) -> Result<Account> {
    let owner = super::normalize_address(owner)?;
    let v = c
        .call("getAddressInformation", json!({"address":owner}))
        .await?;
    let balance = number(&v["balance"])?;
    match v["state"].as_str() {
        Some("uninitialized" | "uninit" | "nonexist") => Ok(Account {
            balance,
            seqno: 0,
            public: None,
            active: false,
        }),
        Some("active") => {
            let decode = |field: &str| -> Result<_> {
                let s = v[field]
                    .as_str()
                    .filter(|s| s.len() <= cell::MAX_BOC * 2)
                    .ok_or_else(|| WalletError::rpc("TON RPC missing bounded wallet code/data"))?;
                let bytes = STANDARD
                    .decode(s)
                    .map_err(|_| WalletError::rpc("TON RPC wallet code/data is not base64"))?;
                cell::decode(&bytes)
                    .map_err(|_| WalletError::rpc("TON RPC wallet code/data has unsupported cells"))
            };
            let code = decode("code")?;
            if alloy::hex::encode(code.hash()) != tx::CODE_HASH {
                return Err(WalletError::rpc(
                    "TON sender is not the supported V4R2 wallet",
                ));
            }
            let data = decode("data")?;
            let (public, seqno) = tx::public_data(&data).map_err(|_| {
                WalletError::rpc("TON wallet data is not V4R2 with an empty plugin dictionary")
            })?;
            if tx::wallet_address(&public) != owner {
                return Err(WalletError::rpc(
                    "TON wallet public key does not derive the requested address",
                ));
            }
            Ok(Account {
                balance,
                seqno,
                public: Some(public),
                active: true,
            })
        }
        _ => Err(WalletError::rpc(
            "TON account is frozen or its state is unsupported",
        )),
    }
}
pub(super) async fn estimate(
    c: &Client,
    owner: &str,
    unsigned: &std::sync::Arc<cell::Cell>,
    active: bool,
) -> Result<(u64, &'static str)> {
    if !active {
        return Ok((super::FEE_RESERVE, "fixed-deployment-reserve"));
    }
    let body = tx::signed_body(unsigned, &[0; 64]);
    let v=c.call("estimateFee",json!({"address":owner,"body":STANDARD.encode(cell::encode(&body)),"init_code":"","init_data":"","ignore_chksig":true})).await?;
    let fees = &v["source_fees"];
    let mut total = 0u64;
    for k in ["in_fwd_fee", "storage_fee", "gas_fee", "fwd_fee"] {
        total = total
            .checked_add(number(&fees[k])?)
            .ok_or_else(|| WalletError::rpc("TON fee estimate overflow"))?;
    }
    let padded = total
        .checked_mul(6)
        .and_then(|x| x.checked_add(4))
        .map(|x| x / 5)
        .ok_or_else(|| WalletError::rpc("TON fee estimate overflow"))?;
    Ok((
        padded.max(super::FEE_RESERVE),
        "rpc-estimate-with-20-percent-margin-and-reserve",
    ))
}
pub(super) fn hash(v: &Value) -> Result<[u8; 32]> {
    let s = v
        .as_str()
        .ok_or_else(|| WalletError::rpc("TON response hash is missing"))?;
    let bytes = if s.len() == 64 {
        alloy::hex::decode(s).ok()
    } else {
        STANDARD.decode(s).ok()
    };
    bytes
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| WalletError::rpc("TON response hash is malformed"))
}
pub(super) fn same_address(v: &Value, raw: &str) -> bool {
    v.as_str()
        .is_some_and(|s| address::parse(s).ok() == address::parse(raw).ok())
}

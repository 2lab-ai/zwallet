//! Error type shared by every command. Each error carries a stable machine code that `--json`
//! output exposes as `errorCode`; the message is human text and must never contain secrets.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    ValidationError,
    WalletExists,
    WalletNotFound,
    WrongPassword,
    WalletLocked,
    KeyExists,
    KeyNotFound,
    ChainUnsupported,
    CapabilityUnsupported,
    InvalidAddress,
    RpcError,
    InsufficientFunds,
    /// The signed transaction was handed to the node but whether it was accepted is unknown.
    BroadcastUncertain,
    /// The verified whitelist does not allow this signature (`.prd/04-whitelist.md`).
    WhitelistDenied,
    /// There is no whitelist the daemon can trust (none yet, or it does not verify against the
    /// sealed anchor): every value-moving signature is refused.
    WhitelistInvalid,
    /// The transaction's fee limit is above the whitelist's cap for its chain, or there is none.
    FeeCapExceeded,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ValidationError => "VALIDATION_ERROR",
            Self::WalletExists => "WALLET_EXISTS",
            Self::WalletNotFound => "WALLET_NOT_FOUND",
            Self::WrongPassword => "WRONG_PASSWORD",
            Self::WalletLocked => "WALLET_LOCKED",
            Self::KeyExists => "KEY_EXISTS",
            Self::KeyNotFound => "KEY_NOT_FOUND",
            Self::ChainUnsupported => "CHAIN_UNSUPPORTED",
            Self::CapabilityUnsupported => "CAPABILITY_UNSUPPORTED",
            Self::InvalidAddress => "INVALID_ADDRESS",
            Self::RpcError => "RPC_ERROR",
            Self::InsufficientFunds => "INSUFFICIENT_FUNDS",
            Self::BroadcastUncertain => "BROADCAST_UNCERTAIN",
            Self::WhitelistDenied => "WHITELIST_DENIED",
            Self::WhitelistInvalid => "WHITELIST_INVALID",
            Self::FeeCapExceeded => "FEE_CAP_EXCEEDED",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct WalletError {
    pub code: ErrorCode,
    pub message: String,
    /// Extra machine-readable fields for the `--json` error object (e.g. `txHash` after a send
    /// whose outcome is unknown). Never secrets.
    pub details: Map<String, Value>,
}

impl WalletError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Map::new(),
        }
    }

    /// Adds a field to the `--json` error object (`error` and `errorCode` cannot be replaced).
    pub fn with_detail(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.details.insert(key.to_string(), value.into());
        self
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationError, message)
    }

    pub fn rpc(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RpcError, message)
    }

    pub fn invalid_address(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidAddress, message)
    }

    /// The wallet has no unlocked key holder; only a human can fix that.
    pub fn locked() -> Self {
        Self::new(
            ErrorCode::WalletLocked,
            "wallet is locked; a human must run `zwallet unlock 24h` (any duration) to allow signing",
        )
    }

    /// JSON body printed on stdout for `--json` failures: `{ error, errorCode, ...details }`.
    pub fn to_json(&self) -> Value {
        let mut out = self.details.clone();
        out.insert("error".into(), Value::from(self.message.as_str()));
        out.insert("errorCode".into(), Value::from(self.code.as_str()));
        Value::Object(out)
    }
}

pub type Result<T> = std::result::Result<T, WalletError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_carries_details_but_never_replaces_error_or_code() {
        let e = WalletError::new(ErrorCode::BroadcastUncertain, "maybe sent")
            .with_detail("txHash", "0xab")
            .with_detail("nonce", 7)
            .with_detail("errorCode", "SPOOFED");
        let v = e.to_json();
        assert_eq!(v["errorCode"], "BROADCAST_UNCERTAIN");
        assert_eq!(v["error"], "maybe sent");
        assert_eq!(v["txHash"], "0xab");
        assert_eq!(v["nonce"], 7);
        assert_eq!(
            WalletError::validation("x").to_json(),
            serde_json::json!({ "error": "x", "errorCode": "VALIDATION_ERROR" })
        );
    }

    #[test]
    fn whitelist_codes_serialize_as_their_names() {
        for code in [
            ErrorCode::WhitelistDenied,
            ErrorCode::WhitelistInvalid,
            ErrorCode::FeeCapExceeded,
        ] {
            let v = serde_json::to_value(code).unwrap();
            assert_eq!(v, code.as_str());
            assert_eq!(serde_json::from_value::<ErrorCode>(v).unwrap(), code);
        }
        assert_eq!(ErrorCode::FeeCapExceeded.as_str(), "FEE_CAP_EXCEEDED");
    }
}

//! Error type shared by every command. Each error carries a stable machine code that `--json`
//! output exposes as `errorCode`; the message is human text and must never contain secrets.

use serde::{Deserialize, Serialize};
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
}

impl WalletError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
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

    /// JSON body printed on stdout for `--json` failures.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "error": self.message, "errorCode": self.code.as_str() })
    }
}

pub type Result<T> = std::result::Result<T, WalletError>;

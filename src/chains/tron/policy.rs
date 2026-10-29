//! Tron signing policy, evaluated only on a whole, strictly decoded raw_data message.
//!
//! Decoder assumptions (`tx::decode_raw`): at most 1024 bytes; fixed reference/address lengths;
//! exact field order at every protobuf level; no unknown, repeated, nonminimal or truncated
//! fields; only contract types 1/31 with exact type URLs; no permissions, memo, auths, scripts,
//! call_value or TRC10 fields; positive int64 native amounts; no native fee_limit. Canonical
//! re-encoding must equal the input. ABI transfer classification stays here: malformed calldata
//! is never interpreted as a permitted transfer. No RPC or file writes belong to this policy.

use super::{
    address, registry,
    tx::{Contract, DecodedTron},
};
use crate::chains::{
    WHITELIST_RULES,
    policy::{ListView, NO_WHITELIST, VERIFY_COMMAND},
};
use crate::core::{
    amount::format_units,
    error::{ErrorCode, Result, WalletError},
    whitelist::Whitelist,
};
use alloy::{hex, primitives::U256};

const CHAIN: &str = "tron";
const DEFAULT_CAP: u64 = 30_000_000;
const MAX_LIFETIME_MS: u64 = 300_000;

fn body(address: &[u8; 21]) -> String {
    format!("0x{}", hex::encode(&address[1..]))
}

struct Denial {
    target: [u8; 21],
    why: String,
    command: Option<String>,
}

impl Denial {
    fn never(target: &[u8; 21], why: &str) -> Self {
        Self {
            target: *target,
            why: why.into(),
            command: None,
        }
    }
    fn error(self, code: ErrorCode, reason: String) -> WalletError {
        let mut error = WalletError::new(code, reason)
            .with_detail("chain", CHAIN)
            .with_detail("counterparty", address::encode(&self.target));
        if let Some(command) = self.command {
            error = error.with_detail("command", command);
        }
        error
    }
}

fn payee(list: &Whitelist, to: &[u8; 21]) -> std::result::Result<(), Denial> {
    let stored = body(to);
    // Exact Tron scope even if a malformed but authenticated legacy list mixed families.
    if list
        .payee(&stored, CHAIN, false)
        .is_some_and(|e| e.chains == [CHAIN])
    {
        return Ok(());
    }
    let shown = address::encode(to);
    Err(match list.by_address(&stored) {
        Some(e) => Denial::never(
            to,
            &format!(
                "{shown} is listed as '{}' ({}) but not as a Tron-only payee",
                e.nickname,
                e.kind.as_str()
            ),
        ),
        None => Denial {
            target: *to,
            why: format!("{shown} is not a payee on tron"),
            command: Some(format!(
                "zwallet whitelist add <nick> {shown} --chains tron"
            )),
        },
    })
}

fn decide(list: &Whitelist, decoded: &DecodedTron) -> std::result::Result<(), Denial> {
    match &decoded.contract {
        Contract::Transfer { to, .. } if *to == decoded.owner => Err(Denial::never(
            to,
            "the node refuses TRX sent to its own sender",
        )),
        Contract::Transfer { to, .. } => payee(list, to),
        Contract::TriggerSmart {
            contract_address, ..
        } => {
            if !registry::is_token(contract_address) {
                return Err(Denial::never(
                    contract_address,
                    "only a Tron registry token may be transferred",
                ));
            }
            let Some((to, _)) = decoded.transfer_call() else {
                return Err(Denial::never(
                    contract_address,
                    "only the exact canonical transfer(address,uint256) call is allowed; approvals, transferFrom and other calls are never signed",
                ));
            };
            if registry::is_token(&to) {
                return Err(Denial::never(
                    &to,
                    "a token transfer to a Tron token contract is never signed",
                ));
            }
            payee(list, &to)
        }
    }
}

/// Owner/clock checks precede list trust, then action, signed amount/calldata limits, and fee.
/// The caller passes its own current clock in milliseconds, never a CLI-provided time.
pub fn check_tx(
    view: &ListView,
    signer: [u8; 21],
    decoded: &DecodedTron,
    now_ms: u64,
) -> Result<()> {
    if decoded.owner != signer {
        return Err(WalletError::validation(
            "refusing to sign: Tron owner is not this key",
        ));
    }
    if decoded.expiration <= decoded.timestamp
        || decoded.expiration.saturating_sub(decoded.timestamp) > MAX_LIFETIME_MS
        || decoded.expiration > now_ms.saturating_add(MAX_LIFETIME_MS)
    {
        return Err(WalletError::validation(
            "refusing to sign: Tron expiration must follow its timestamp and be within 300 seconds of it and the daemon's clock",
        ));
    }
    let list = match view {
        ListView::Usable(list) => *list,
        _ => {
            let empty = Whitelist::empty(&WHITELIST_RULES);
            let mut denial = match decide(&empty, decoded) {
                Err(denial) => denial,
                Ok(()) => Denial::never(
                    &target(decoded),
                    "no Tron action is allowed without a whitelist",
                ),
            };
            let message = match view {
                ListView::Unusable(reason) => {
                    denial.command = None;
                    format!(
                        "refusing to sign: the whitelist cannot be trusted: {reason}; nothing is signed. A human checks it with `{VERIFY_COMMAND}`"
                    )
                }
                _ => match &denial.command {
                    Some(command) => format!(
                        "refusing to sign: {NO_WHITELIST}; nothing is signed on tron. A human can allow this payee with `{command}`"
                    ),
                    None => format!(
                        "refusing to sign: {NO_WHITELIST}; nothing is signed on tron; {}",
                        denial.why
                    ),
                },
            };
            return Err(denial.error(ErrorCode::WhitelistInvalid, message));
        }
    };
    decide(list, decoded).map_err(|denial| {
        let message = match &denial.command {
            Some(command) => format!(
                "refusing to sign: {}; a human can allow it with `{command}`",
                denial.why
            ),
            None => format!("refusing to sign: {}", denial.why),
        };
        denial.error(ErrorCode::WhitelistDenied, message)
    })?;
    check_limits(list, decoded)?;
    let fee = U256::from(decoded.fee_limit.unwrap_or(0));
    let cap = list.policy.max_fee_wei(CHAIN);
    if cap.is_some_and(|cap| fee <= cap) {
        return Ok(());
    }
    let required = if cap.is_none() {
        fee.max(U256::from(DEFAULT_CAP))
    } else {
        fee
    };
    let command = format!(
        "zwallet whitelist fee-cap tron {}",
        format_units(required, 6)
    );
    let why = match cap {
        Some(cap) => format!(
            "fee_limit {} TRX is above the whitelist cap {} TRX",
            format_units(fee, 6),
            format_units(cap, 6)
        ),
        None => "the whitelist has no readable fee cap for tron".into(),
    };
    let mut error = WalletError::new(
        ErrorCode::FeeCapExceeded,
        format!("refusing to sign: {why}; a human can allow it with `{command}`"),
    )
    .with_detail("chain", CHAIN)
    .with_detail("counterparty", address::encode(&target(decoded)))
    .with_detail("command", command)
    .with_detail("feeWei", fee.to_string());
    if let Some(cap) = cap {
        error = error.with_detail("capWei", cap.to_string());
    }
    Err(error)
}

fn target(decoded: &DecodedTron) -> [u8; 21] {
    match decoded.contract {
        Contract::Transfer { to, .. } => to,
        Contract::TriggerSmart {
            contract_address, ..
        } => contract_address,
    }
}

/// Historical maxNativeWei stores sun on Tron; token limits store smallest token units.
fn check_limits(list: &Whitelist, decoded: &DecodedTron) -> Result<()> {
    let Some(limits) = list.policy.transaction_limits.get(CHAIN) else {
        return Ok(());
    };
    let refused = |why: &str| {
        WalletError::new(
            ErrorCode::WhitelistDenied,
            format!("refusing to sign: per-transaction {why} exceeded on tron"),
        )
        .with_detail("chain", CHAIN)
        .with_detail("counterparty", address::encode(&target(decoded)))
    };
    let parse = |value: &str| {
        U256::from_str_radix(value, 10).map_err(|_| refused("amount cap is invalid or"))
    };
    let (native, calldata) = match &decoded.contract {
        Contract::Transfer { amount_sun, .. } => (U256::from(*amount_sun), 0),
        Contract::TriggerSmart { data, .. } => (U256::ZERO, data.len()),
    };
    if let Some(cap) = &limits.max_native_wei
        && native > parse(cap)?
    {
        return Err(refused("native value cap (sun)"));
    }
    if limits
        .max_calldata_bytes
        .is_some_and(|cap| calldata as u64 > u64::from(cap))
    {
        return Err(refused("calldata byte cap (not a bandwidth fee cap)"));
    }
    if let Contract::TriggerSmart {
        contract_address, ..
    } = &decoded.contract
        && let Some((_, amount)) = decoded.transfer_call()
        && let Some(cap) = limits.max_token_base_units.get(&body(contract_address))
        && amount > parse(cap)?
    {
        return Err(refused("token amount cap (smallest base units)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::rejected_codec_corpus;

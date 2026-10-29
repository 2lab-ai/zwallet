//! Tron transfer preflight and independent signer verification. The fee maximum assumes that
//! another transaction spends all currently available resources first; prices are node state.
use super::{
    address, registry, rpc,
    tx::{self, Contract, DecodedTron},
};
use crate::chains::{Net, PreparedTx, SignedTx, TransferRequest};
use crate::core::{
    amount::{format_units, parse_units},
    error::{ErrorCode, Result, WalletError},
};
use alloy::{
    hex,
    primitives::{Signature, U256},
};
use serde_json::{Value, json};

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Exact millisecond RFC3339, including the journal's expiration.
pub fn expires_at(ms: u64) -> String {
    let seconds = crate::core::time::rfc3339_utc(ms / 1000);
    format!("{}.{:03}Z", seconds.trim_end_matches('Z'), ms % 1000)
}
pub fn parse_expiration(text: &str) -> Option<u64> {
    if text.len() == 24 && text.as_bytes()[19] == b'.' && text.ends_with('Z') {
        let millis: u64 = text.get(20..23)?.parse().ok()?;
        let seconds = crate::core::time::parse_rfc3339_utc(&format!("{}Z", text.get(..19)?))?;
        seconds.checked_mul(1000)?.checked_add(millis)
    } else {
        crate::core::time::parse_rfc3339_utc(text)?.checked_mul(1000)
    }
}

/// A numeric protobuf int64 from a node. No string coercion, truncation or signed wrap.
pub(super) fn number(v: Option<&Value>, name: &str, default: Option<u64>) -> Result<u64> {
    match v {
        None => default.ok_or_else(|| WalletError::rpc(format!("the node omitted {name}"))),
        Some(v) => v.as_u64().filter(|n| *n <= i64::MAX as u64).ok_or_else(|| {
            WalletError::rpc(format!("the node's {name} is not a nonnegative int64"))
        }),
    }
}

pub(super) struct Head {
    pub number: u64,
    pub id: [u8; 32],
    pub timestamp: u64,
}
pub(super) async fn head(c: &rpc::TronClient) -> Result<Head> {
    let b = c.post("getblock", &json!({"detail": false})).await?;
    let id: [u8; 32] = b
        .get("blockID")
        .and_then(Value::as_str)
        .and_then(|s| hex::decode(s).ok())
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| WalletError::rpc("the node's head has no 32-byte blockID"))?;
    let raw = b
        .get("block_header")
        .and_then(|v| v.get("raw_data"))
        .ok_or_else(|| WalletError::rpc("the node's head has no block_header.raw_data"))?;
    let number = number(raw.get("number"), "block number", None)?;
    let timestamp = self::number(raw.get("timestamp"), "block timestamp", None)?;
    if id[..8] != number.to_be_bytes() || timestamp == 0 {
        return Err(WalletError::rpc(
            "the node's head blockID and block number disagree, or its time is zero",
        ));
    }
    Ok(Head {
        number,
        id,
        timestamp,
    })
}
fn build_raw(
    number: u64,
    id: [u8; 32],
    timestamp: u64,
    owner: [u8; 21],
    contract: Contract,
    fee_limit: Option<u64>,
) -> Result<DecodedTron> {
    Ok(DecodedTron {
        ref_block_bytes: number.to_be_bytes()[6..].try_into().unwrap(),
        ref_block_hash: id[8..16].try_into().unwrap(),
        timestamp,
        expiration: timestamp
            .checked_add(60_000)
            .ok_or_else(|| WalletError::rpc("block timestamp overflow"))?,
        owner,
        contract,
        fee_limit,
    })
}

#[derive(Clone, Copy)]
struct Prices {
    bandwidth: u64,
    energy: u64,
    activation: u64,
    new_bandwidth: u64,
    rate: u64,
    max_fee: u64,
}
impl Prices {
    async fn read(c: &rpc::TronClient) -> Result<Self> {
        let reply = c.post("getchainparameters", &json!({})).await?;
        let entries = reply
            .get("chainParameter")
            .and_then(Value::as_array)
            .ok_or_else(|| WalletError::rpc("the node omitted chainParameter"))?;
        let price = |key: &str| -> Result<u64> {
            let mut entries = entries
                .iter()
                .filter(|v| v.get("key").and_then(Value::as_str) == Some(key));
            let value = entries
                .next()
                .ok_or_else(|| WalletError::rpc(format!("the node omitted {key}")))?;
            if entries.next().is_some() {
                return Err(WalletError::rpc(format!("the node repeated {key}")));
            }
            // protobuf omits the value of zero-valued parameters.
            number(value.get("value"), key, Some(0))
        };
        let p = Self {
            bandwidth: price("getTransactionFee")?,
            energy: price("getEnergyFee")?,
            activation: price("getCreateNewAccountFeeInSystemContract")?,
            new_bandwidth: price("getCreateAccountFee")?,
            rate: price("getCreateNewAccountBandwidthRate")?,
            max_fee: price("getMaxFeeLimit")?,
        };
        if p.energy == 0 || p.rate == 0 || p.max_fee == 0 {
            return Err(WalletError::rpc(
                "the node supplied a zero energy price, bandwidth rate or maximum fee limit",
            ));
        }
        Ok(p)
    }
}
#[derive(Default, Clone, Copy)]
struct Resources {
    staked: u64,
    free: u64,
    energy: u64,
}
impl Resources {
    async fn read(c: &rpc::TronClient, from: &str) -> Result<Self> {
        let r = c
            .post(
                "getaccountresource",
                &json!({"address": from, "visible": true}),
            )
            .await?;
        let remaining = |limit, used| -> Result<u64> {
            Ok(number(r.get(limit), limit, Some(0))?.saturating_sub(number(
                r.get(used),
                used,
                Some(0),
            )?))
        };
        Ok(Self {
            staked: remaining("NetLimit", "NetUsed")?,
            free: remaining("freeNetLimit", "freeNetUsed")?,
            energy: remaining("EnergyLimit", "EnergyUsed")?,
        })
    }
}
fn bandwidth_size(raw: &[u8]) -> u64 {
    let mut n = raw.len();
    let mut length_bytes = 1;
    while n >= 128 {
        length_bytes += 1;
        n >>= 7;
    }
    (1 + length_bytes + raw.len() + 1 + 1 + 65 + 64) as u64
}
/// Integer review bounds calculated before any display formatting. `max_cost_sun` includes
/// the native payment; the maximum fee assumes all currently available resources are spent.
#[derive(Debug)]
pub struct PreparedTronTransfer {
    pub prepared: PreparedTx,
    pub estimated_cost_sun: U256,
    pub max_cost_sun: U256,
    pub max_fee_sun: U256,
    pub balance_sun: U256,
}
struct FeeEstimate {
    fields: Value,
    estimated: U256,
    maximum: U256,
    max_fee: U256,
}
#[allow(clippy::too_many_arguments)]
fn fees(
    bytes: u64,
    amount: U256,
    token: bool,
    payee: &rpc::Account,
    p: Prices,
    r: Resources,
    energy: u64,
    fee: u64,
    block: u64,
) -> FeeEstimate {
    let new = !token && !payee.activated;
    let required = U256::from(bytes) * U256::from(if new { p.rate } else { 1 });
    let source = if U256::from(r.staked) >= required {
        "staked"
    } else if !new && r.free >= bytes {
        "free"
    } else {
        "burn"
    };
    let normal = U256::from(bytes) * U256::from(p.bandwidth);
    let bandwidth = if source != "burn" {
        U256::ZERO
    } else if new {
        U256::from(p.new_bandwidth)
    } else {
        normal
    };
    let activation = if new {
        U256::from(p.activation)
    } else {
        U256::ZERO
    };
    let energy_sun = U256::from(energy.saturating_sub(r.energy)) * U256::from(p.energy);
    let native = if token { U256::ZERO } else { amount };
    let estimate = native + bandwidth + activation + energy_sun;
    let maximum = native
        + if token {
            normal + U256::from(fee)
        } else if !payee.activated || payee.contract {
            normal.max(U256::from(p.activation) + U256::from(p.new_bandwidth))
        } else {
            normal
        };
    let fields = json!({"bandwidthBytes": bytes, "bandwidthSource": source, "bandwidthSun": bandwidth.to_string(),
        "activationSun": activation.to_string(), "energy": energy, "energySun": energy_sun.to_string(),
        "feeLimit": fee.to_string(), "estimatedCostSun": estimate.to_string(), "maxCostSun": maximum.to_string(),
        "estimatedCost": format_units(estimate, 6), "maxCost": format_units(maximum, 6), "pricesAtBlock": block});
    FeeEstimate {
        fields,
        estimated: estimate,
        maximum,
        max_fee: maximum - native,
    }
}

pub fn validate_flags(req: &TransferRequest) -> Result<()> {
    if req.nonce.is_some()
        || req.bump.is_some()
        || req.replace.is_some()
        || req.allow_pending
        || req.max_fee_per_gas.is_some()
    {
        return Err(WalletError::validation(
            "Tron has no nonce; --nonce, --replace, --bump, --allow-pending and --max-fee-gwei do not apply",
        ));
    }
    validate_token(req.token.as_deref())
}
pub fn validate_token(token: Option<&str>) -> Result<()> {
    if token.is_some_and(|t| registry::by_symbol(t).is_none()) {
        return Err(WalletError::validation(
            "only registry tokens are sent on Tron: --token usdt",
        ));
    }
    Ok(())
}

pub async fn prepare_transfer(net: &Net, req: &TransferRequest) -> Result<PreparedTx> {
    Ok(prepare_review(net, req).await?.prepared)
}

/// The same CLI preflight with integer cost bounds retained for immutable TUI reviews.
pub async fn prepare_review(net: &Net, req: &TransferRequest) -> Result<PreparedTronTransfer> {
    validate_flags(req)?;
    let owner = address::parse(&req.from)?;
    let to = address::parse(&req.to)?;
    let amount = parse_units(&req.amount, 6)?;
    if amount.is_zero() {
        return Err(WalletError::validation(
            "the amount must be greater than zero",
        ));
    }
    let token = req.token.as_deref().and_then(registry::by_symbol);
    let native = if token.is_none() {
        u64::try_from(amount)
            .ok()
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| WalletError::validation("TRX amount exceeds int64 sun"))?
    } else {
        0
    };
    let c = rpc::connect(net).await?;
    let account = rpc::account(&c, &req.from).await?;
    if !account.activated {
        return Err(WalletError::new(
            ErrorCode::InsufficientFunds,
            "this Tron address holds no TRX yet; send it TRX first, for bandwidth and energy",
        ));
    }
    if let Some(warning) = account.warning {
        return Err(WalletError::validation(format!(
            "{warning}; refusing to sign"
        )));
    }
    let resources = Resources::read(&c, &req.from).await?;
    let prices = Prices::read(&c).await?;
    let payee = if token.is_none() {
        rpc::account(&c, &req.to).await?
    } else {
        rpc::Account {
            activated: true,
            balance: 0,
            contract: false,
            warning: None,
        }
    };
    let (contract, energy, fee_limit, token_balance) = if let Some(t) = token {
        let balance = rpc::trc20_balance(&c, t.address, &req.from).await?;
        if balance < amount {
            return Err(WalletError::new(
                ErrorCode::InsufficientFunds,
                "insufficient USDT balance",
            )
            .with_detail("shortfall", (amount - balance).to_string()));
        }
        let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
        data.extend_from_slice(&[0; 12]);
        data.extend_from_slice(&to[1..]);
        data.extend_from_slice(&amount.to_be_bytes::<32>());
        let simulated = rpc::constant_reply(
            &c,
            &req.from,
            t.address,
            "transfer(address,uint256)",
            &data[4..],
        )
        .await?;
        let energy = number(simulated.get("energy_used"), "energy_used", None)?;
        let fee = ((U256::from(energy) * U256::from(prices.energy) * U256::from(6)
            + U256::from(4))
            / U256::from(5))
        .max(U256::from(1_000_000));
        if fee > U256::from(prices.max_fee) || fee > U256::from(i64::MAX as u64) {
            return Err(WalletError::new(
                ErrorCode::FeeCapExceeded,
                "simulated energy needs a fee_limit above the chain maximum; nothing was signed",
            )
            .with_detail("feeWei", fee.to_string())
            .with_detail("capWei", prices.max_fee.to_string()));
        }
        (
            Contract::TriggerSmart {
                contract_address: address::parse(t.address)?,
                data,
            },
            energy,
            Some(fee.to::<u64>()),
            Some(balance),
        )
    } else {
        (
            Contract::Transfer {
                to,
                amount_sun: native,
            },
            0,
            None,
            None,
        )
    };
    // Read the reference after preflight; a slow simulation must not spend the transaction's lifetime.
    let block = head(&c).await?;
    let now = now_ms();
    if now.saturating_sub(block.timestamp) >= 60_000 || block.timestamp > now.saturating_add(30_000)
    {
        return Err(WalletError::rpc(format!(
            "the node's head block is {} s old or ahead of this clock; ask another node with --rpc",
            now.saturating_sub(block.timestamp) / 1000
        )));
    }
    let decoded = build_raw(
        block.number,
        block.id,
        block.timestamp,
        owner,
        contract,
        fee_limit,
    )?;
    let raw = tx::encode_raw(&decoded)?;
    let costs = fees(
        bandwidth_size(&raw),
        amount,
        token.is_some(),
        &payee,
        prices,
        resources,
        energy,
        fee_limit.unwrap_or(0),
        block.number,
    );
    let maximum = costs.maximum;
    let balance = U256::from(account.balance);
    let shortfall = maximum.saturating_sub(balance);
    if !shortfall.is_zero() {
        return Err(WalletError::new(ErrorCode::InsufficientFunds, format!("insufficient TRX for maxCostSun {maximum}; shortfall {shortfall} sun; nothing was signed"))
            .with_detail("maxCostSun", maximum.to_string()).with_detail("shortfall", shortfall.to_string()).with_detail("fees", costs.fields));
    }
    let mut fields = json!({"amount": format_units(amount, 6), "amountRaw": amount.to_string(), "symbol": token.map_or("TRX", |t| t.symbol),
        "decimals": 6, "contractType": if token.is_some() {"TriggerSmartContract"} else {"TransferContract"},
        "refBlock": {"number": block.number, "id": hex::encode(block.id)}, "timestamp": decoded.timestamp,
        "expiration": decoded.expiration, "expiresAt": expires_at(decoded.expiration), "fees": costs.fields,
        "nativeBalance": format_units(balance, 6), "sufficientFunds": true, "shortfall": "0"}).as_object().unwrap().clone();
    if let Some(t) = token {
        fields.insert("token".into(), json!(t.address));
        fields.insert("feeLimit".into(), json!(fee_limit.unwrap().to_string()));
        fields.insert(
            "tokenBalance".into(),
            json!(format_units(token_balance.unwrap(), 6)),
        );
    }
    Ok(PreparedTronTransfer {
        prepared: PreparedTx {
            unsigned: json!(format!("0x{}", hex::encode(raw))),
            fields,
            max_gas_cost: costs.max_fee,
            l1_fee: None,
        },
        estimated_cost_sun: costs.estimated,
        max_cost_sun: costs.maximum,
        max_fee_sun: costs.max_fee,
        balance_sun: balance,
    })
}

/// Compare the raw payload to the intent before trusting anything the daemon says about it.
/// Even a valid signature by this key is not authority to send different bytes.
pub fn inspect_signed(signed: &Value, expected: Option<&Value>) -> Result<SignedTx> {
    let bytes = |key: &str, max: usize| -> Result<Vec<u8>> {
        signed
            .get(key)
            .and_then(Value::as_str)
            .and_then(|s| s.strip_prefix("0x"))
            .filter(|s| s.len() <= max * 2)
            .and_then(|s| hex::decode(s).ok())
            .ok_or_else(|| WalletError::validation(format!("signer returned malformed {key}")))
    };
    let raw = bytes("rawData", tx::MAX_RAW_BYTES)?;
    if let Some(expected) = expected {
        let want = expected
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .and_then(|s| hex::decode(s).ok());
        if want.as_deref() != Some(raw.as_slice()) {
            return Err(WalletError::validation(
                "the key daemon signed different rawData; not broadcasting",
            ));
        }
    }
    let decoded = tx::decode_raw(&raw)?;
    let hash = hex::encode(tx::txid(&raw));
    if signed.get("txHash").and_then(Value::as_str) != Some(hash.as_str()) {
        return Err(WalletError::validation(
            "the key daemon's txHash differs from sha256(rawData); not broadcasting",
        ));
    }
    let signature = bytes("signature", 65)?;
    if signature.len() != 65
        || signature[64] > 1
        || Signature::from_raw(&signature)
            .ok()
            .is_none_or(|s| s.normalize_s().is_some())
    {
        return Err(WalletError::validation(
            "the key daemon returned a noncanonical signature (expected 65 bytes, low s, v 0 or 1)",
        ));
    }
    let recovered = tx::recover(&raw, &signature)?;
    if recovered != decoded.owner {
        return Err(WalletError::validation(
            "the key daemon signature is from another owner; not broadcasting",
        ));
    }
    Ok(SignedTx {
        raw: hex::encode(tx::encode_signed(&raw, &signature)?),
        tx_hash: hash,
        signer: address::encode(&recovered),
        nonce: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_time_and_exact_expiration_are_built_locally() {
        let owner = address::parse("TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6").unwrap();
        let to = address::parse("TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C").unwrap();
        let raw = build_raw(
            0x123456,
            [0x77; 32],
            1_800_000_000_123,
            owner,
            Contract::Transfer { to, amount_sun: 1 },
            None,
        )
        .unwrap();
        assert_eq!(raw.ref_block_bytes, [0x34, 0x56]);
        assert_eq!(raw.ref_block_hash, [0x77; 8]);
        assert_eq!(raw.timestamp, 1_800_000_000_123);
        assert_eq!(raw.expiration, 1_800_000_060_123);
        assert_eq!(
            parse_expiration(&expires_at(raw.expiration)),
            Some(raw.expiration)
        );
    }
    #[test]
    fn dynamic_activation_branches_and_resources_do_not_reduce_the_maximum() {
        let p = Prices {
            bandwidth: 2000,
            energy: 100,
            activation: 2_000_000,
            new_bandwidth: 300_000,
            rate: 3,
            max_fee: 15_000_000_000,
        };
        for (activated, contract, maximum) in [
            (false, false, "3300000"),
            (true, false, "1534000"),
            (true, true, "3300000"),
        ] {
            let payee = rpc::Account {
                activated,
                contract,
                balance: 0,
                warning: None,
            };
            for staked in [0, 801] {
                let f = fees(
                    267,
                    U256::from(1_000_000),
                    false,
                    &payee,
                    p,
                    Resources {
                        staked,
                        free: 600,
                        energy: 0,
                    },
                    0,
                    0,
                    42,
                );
                assert_eq!(f.fields["maxCostSun"], maximum, "{}", f.fields);
                if !activated {
                    assert_eq!(
                        f.fields["estimatedCostSun"],
                        if staked == 0 { "3300000" } else { "3000000" }
                    );
                }
            }
        }
        let payee = rpc::Account {
            activated: true,
            contract: false,
            balance: 0,
            warning: None,
        };
        let f = fees(
            345,
            U256::from(1_000_000),
            true,
            &payee,
            p,
            Resources {
                energy: 100,
                ..Default::default()
            },
            1000,
            1_000_000,
            42,
        );
        assert_eq!(f.fields["maxCostSun"], "1690000");
        assert_eq!(f.maximum, U256::from(1_690_000));
        assert_eq!(f.estimated, U256::from(780_000));
        assert_eq!(f.fields["estimatedCostSun"], "780000");
    }
}

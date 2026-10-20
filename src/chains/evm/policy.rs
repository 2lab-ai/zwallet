//! The signing policy (`.prd/04-whitelist.md` §Daemon enforcement): what may be signed, decided
//! from the daemon's own decoding of the bytes ([`DecodedUnsigned`]) against the whitelist.
//!
//! The daemon calls [`check_tx`] in `Evm::sign_tx` right after `tx::decode_unsigned` and the
//! chainId check and before `tx::sign_decoded`, and [`check_message`] before every EIP-191
//! signature, with the list that verifies against the anchor sealed in wallet.json
//! ([`ListView::Usable`]), or with why there is none: no whitelist yet ([`ListView::NoList`]) or
//! nothing that can be trusted ([`ListView::Unusable`]). The CLI may call the same functions on the
//! unverified list to explain a refusal before asking the daemon; only the daemon's call decides.
//!
//! Every refusal says what it is about (`counterparty`, `chain`) and, when one edit of the list
//! allows it, the exact `zwallet whitelist …` command a human runs (`command`). A list that cannot
//! be trusted gets no such command: it is checked first (`zwallet whitelist list --verify`).
//!
//! No signature escapes a fee cap: the list's, or while there is no list yet the built-in default
//! for the chain (`registry::DEFAULT_FEE_CAPS_WEI`), which only a self-transfer can reach. A list
//! that exists but cannot be trusted signs nothing at all, self-transfers included: breaking the
//! list must never be a way around its own, stricter, signed cap.

use crate::chains::Chain;
use crate::chains::evm::tx::{Action, DecodedUnsigned};
use crate::chains::evm::{config, registry};
use crate::core::amount::format_units;
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::whitelist::{EntryKind, Whitelist};
use alloy::hex;
use alloy::primitives::{Address, U256};

/// Why a wallet without a whitelist refuses: its first edit creates one.
pub const NO_WHITELIST: &str = "this wallet has no whitelist yet; the first `zwallet whitelist add \
                                <nick> <addr> --chains <chain>` creates it";

/// The command that lets `sign-message` sign.
pub const MESSAGE_SIGNING_COMMAND: &str = "zwallet whitelist allow-message-signing on";

/// What a human runs when the list cannot be trusted: it shows whether it verifies, and why not.
pub const VERIFY_COMMAND: &str = "zwallet whitelist list --verify";

/// The list a signature is checked against.
#[derive(Debug)]
pub enum ListView<'a> {
    /// A list that verified against the sealed anchor (the daemon), or the unverified list the CLI
    /// explains a dry-run with.
    Usable(&'a Whitelist),
    /// This wallet has no whitelist yet (wallet.json seals no anchor). Only a native transfer to
    /// the signing key's own address is signed, under the built-in fee cap. Refusals name the edit
    /// that would allow the request; that first edit creates the list.
    NoList,
    /// There is a list or an anchor, but nothing that can be trusted, and why: it does not
    /// verify, cannot be read (in time), or the sealed anchor is malformed. Nothing is signed,
    /// self-transfers included. Refusals name no edit (one would rebuild the list around whatever
    /// the request asks for); they point to [`VERIFY_COMMAND`].
    Unusable(String),
}

/// Decides one decoded unsigned transaction against the list. `Ok(())` = may sign.
///
/// With a usable list: the fee limit must be within the list's cap for the chain
/// (`FEE_CAP_EXCEEDED`, also when there is no cap), then a native transfer to `signer` itself
/// passes and anything else must be an action the list allows (`WHITELIST_DENIED`). With no list
/// yet, only a native transfer to `signer` passes, under the chain's built-in default fee cap
/// (`FEE_CAP_EXCEEDED` above it, or without one); anything else is `WHITELIST_INVALID`. With a
/// list that cannot be trusted, everything is `WHITELIST_INVALID`.
pub fn check_tx(
    view: &ListView,
    chain: &Chain,
    signer: &Address,
    decoded: &DecodedUnsigned,
) -> Result<()> {
    let action = decoded.action();
    let to_self = matches!(action, Action::NativeTransfer { to, .. } if to == signer);
    let list = match view {
        ListView::Usable(list) => *list,
        ListView::NoList if to_self => {
            return check_fee(
                CapSource::BuiltIn,
                default_fee_cap(chain.key),
                chain,
                decoded,
            );
        }
        ListView::NoList | ListView::Unusable(_) => return Err(invalid(view, chain, action)),
    };
    check_fee(
        CapSource::List,
        list.policy.max_fee_wei(chain.key),
        chain,
        decoded,
    )?;
    if to_self {
        return Ok(());
    }
    decide(list, chain, action).map_err(|d| {
        let message = match &d.fix {
            Fix::Add(c) => format!(
                "refusing to sign: {}; a human can allow it with `{c}`",
                d.why
            ),
            Fix::Widen => format!("refusing to sign: {}", d.why),
            Fix::Never => format!(
                "refusing to sign: {}; no whitelist entry can allow this",
                d.why
            ),
        };
        d.error(ErrorCode::WhitelistDenied, chain, message)
    })
}

/// The `sign-message` gate: refused unless the list verifies and has `allowMessageSigning` on
/// (some contracts take an EIP-191 signature as spending authority).
pub fn check_message(view: &ListView) -> Result<()> {
    let refuse = |code, message: String| WalletError::new(code, message);
    Err(match view {
        ListView::Usable(list) if list.policy.allow_message_signing => return Ok(()),
        ListView::Usable(_) => refuse(
            ErrorCode::WhitelistDenied,
            format!(
                "refusing to sign a message: the whitelist does not allow message signing (some \
                 contracts take an EIP-191 signature as spending authority); a human can turn it \
                 on with `{MESSAGE_SIGNING_COMMAND}`"
            ),
        )
        .with_detail("command", MESSAGE_SIGNING_COMMAND),
        ListView::NoList => refuse(
            ErrorCode::WhitelistInvalid,
            format!(
                "refusing to sign a message: {NO_WHITELIST}. Message signing needs a whitelist \
                 that allows it: `{MESSAGE_SIGNING_COMMAND}`"
            ),
        )
        .with_detail("command", MESSAGE_SIGNING_COMMAND),
        ListView::Unusable(reason) => refuse(
            ErrorCode::WhitelistInvalid,
            format!(
                "refusing to sign a message: the whitelist cannot be trusted: {reason}. A human \
                 checks it with `{VERIFY_COMMAND}`"
            ),
        ),
    })
}

/// What, if anything, lets the action table allow a refused action.
enum Fix {
    /// This one edit (adding an entry).
    Add(String),
    /// Changing the entry the message names (another chain or selector). No command: the CLI
    /// edits an entry by removing it and adding it again, two edits a human chooses.
    Widen,
    /// No entry can.
    Never,
}

/// Why the action table refuses, about which address, and what would allow it.
struct Denial {
    counterparty: Address,
    why: String,
    fix: Fix,
}

impl Denial {
    fn never(counterparty: &Address, why: String) -> Self {
        Self {
            counterparty: *counterparty,
            why,
            fix: Fix::Never,
        }
    }

    fn error(self, code: ErrorCode, chain: &Chain, message: String) -> WalletError {
        let e = WalletError::new(code, message)
            .with_detail("counterparty", self.counterparty.to_checksum(None))
            .with_detail("chain", chain.key);
        match self.fix {
            Fix::Add(c) => e.with_detail("command", c),
            Fix::Widen | Fix::Never => e,
        }
    }
}

/// Without a usable list: `WHITELIST_INVALID`. With no list yet, naming the edit the action
/// would need (the first edit creates the list); with an untrusted one, pointing to
/// [`VERIFY_COMMAND`] and naming no edit.
fn invalid(view: &ListView, chain: &Chain, action: &Action) -> WalletError {
    let until = "Until a human signs a whitelist, only transfers to this key's own address are \
                 signed, under the built-in fee cap";
    let empty = Whitelist::empty(&registry::WHITELIST_RULES);
    let d = match decide(&empty, chain, action) {
        Err(d) => d,
        // Allowed without any entry (a revoke): any signed list allows it.
        Ok(()) => Denial {
            counterparty: target(action),
            why: String::new(),
            fix: Fix::Widen,
        },
    };
    let message = match (view, &d.fix) {
        (ListView::Unusable(reason), _) => format!(
            "refusing to sign: the whitelist cannot be trusted: {reason}. Nothing is signed, not \
             even a transfer to this key's own address, until it verifies again; a human checks \
             it with `{VERIFY_COMMAND}`"
        ),
        (_, Fix::Add(c)) => {
            format!("refusing to sign: {NO_WHITELIST}. {until}; to allow this one: `{c}`")
        }
        (_, Fix::Widen) => format!("refusing to sign: {NO_WHITELIST}. {until}"),
        (_, Fix::Never) => format!(
            "refusing to sign: {NO_WHITELIST}. {until}; and no whitelist entry can allow this one: \
             {}",
            d.why
        ),
    };
    let d = match view {
        ListView::Unusable(_) => Denial {
            fix: Fix::Never,
            ..d
        },
        _ => d,
    };
    d.error(ErrorCode::WhitelistInvalid, chain, message)
}

/// The address a transaction calls or pays.
fn target(action: &Action) -> Address {
    match action {
        Action::NativeTransfer { to, .. } => *to,
        Action::TransferCall { token, .. } => *token,
        Action::ApproveCall { contract, .. }
        | Action::TransferFromCall { contract, .. }
        | Action::ContractCall { contract, .. } => *contract,
    }
}

/// The built-in fee cap of chain `key` (`registry::DEFAULT_FEE_CAPS_WEI`): part of the binary,
/// so nothing in the home can raise it.
fn default_fee_cap(key: &str) -> Option<U256> {
    registry::DEFAULT_FEE_CAPS_WEI
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, wei)| U256::from_str_radix(wei, 10).ok())
}

/// Where a fee cap comes from.
#[derive(Clone, Copy)]
enum CapSource {
    /// The verified list's `maxFeeWei` for the chain.
    List,
    /// The built-in default, while there is no list yet.
    BuiltIn,
}

/// `gasLimit × maxFeePerGas` (type 2) or `× gasPrice` (legacy) against `cap`: the usable list's
/// cap for the chain, or the built-in default while there is no list yet. A missing cap refuses
/// too; either way the first or next `fee-cap` edit is the fix.
fn check_fee(
    source: CapSource,
    cap: Option<U256>,
    chain: &Chain,
    decoded: &DecodedUnsigned,
) -> Result<()> {
    let fee = U256::from(decoded.gas_limit()) * U256::from(decoded.fees().max_price());
    if cap.is_some_and(|cap| fee <= cap) {
        return Ok(());
    }
    let units = |wei: U256| format!("{} {}", format_units(wei, chain.decimals), chain.symbol);
    let set_cap = format!(
        "zwallet whitelist fee-cap {} {}",
        chain.key,
        format_units(fee, chain.decimals)
    );
    let limit = format!(
        "the fee limit {} (gasLimit {} x {} wei)",
        units(fee),
        decoded.gas_limit(),
        decoded.fees().max_price()
    );
    let why = match (source, cap) {
        (CapSource::List, Some(cap)) => format!(
            "{limit} is above the whitelist's cap of {} on {}",
            units(cap),
            chain.key
        ),
        (CapSource::List, None) => format!(
            "the whitelist has no fee cap for {} (or an unreadable one), and {limit}",
            chain.key
        ),
        (CapSource::BuiltIn, Some(cap)) => format!(
            "{limit} is above the built-in cap of {} on {} that applies while there is no \
             whitelist; the first fee-cap edit creates the list with its own cap",
            units(cap),
            chain.key
        ),
        (CapSource::BuiltIn, None) => format!(
            "there is no built-in fee cap for {} and no whitelist yet, and {limit}",
            chain.key
        ),
    };
    let mut e = WalletError::new(
        ErrorCode::FeeCapExceeded,
        format!("refusing to sign: {why}; a human can allow it with `{set_cap}`"),
    )
    .with_detail("counterparty", decoded.to().to_checksum(None))
    .with_detail("chain", chain.key)
    .with_detail("command", set_cap)
    .with_detail("feeWei", fee.to_string());
    if let Some(cap) = cap {
        e = e.with_detail("capWei", cap.to_string());
    }
    Err(e)
}

/// The action table (`.prd/04-whitelist.md` §Daemon enforcement).
fn decide(list: &Whitelist, chain: &Chain, action: &Action) -> std::result::Result<(), Denial> {
    let chain_id = config(chain).chain_id;
    match action {
        Action::NativeTransfer { to, .. } => payee(list, chain, to),
        Action::TransferCall { token, to, .. } => {
            if registry::lookup(chain_id, token).is_none() {
                return Err(Denial::never(
                    token,
                    format!(
                        "{} is not a registry token on {}; only the built-in USDC, USDT, DAI and \
                         WETH deployments are transferred",
                        token.to_checksum(None),
                        chain.key
                    ),
                ));
            }
            if registry::is_token_anywhere(to) {
                return Err(Denial::never(
                    to,
                    format!(
                        "{} is a token contract; a token transfer to one is never signed",
                        to.to_checksum(None)
                    ),
                ));
            }
            payee(list, chain, to)
        }
        Action::ApproveCall {
            contract,
            spender,
            value,
        } => {
            if registry::lookup(chain_id, contract).is_none() {
                return Err(Denial::never(
                    contract,
                    format!(
                        "approve on {}, which is not a registry token on {}",
                        contract.to_checksum(None),
                        chain.key
                    ),
                ));
            }
            if !value.is_zero() {
                return Err(Denial::never(
                    spender,
                    format!(
                        "approvals are revoke-only: this approves {value} to {} on {}; only an \
                         approve of 0 is signed",
                        spender.to_checksum(None),
                        contract.to_checksum(None)
                    ),
                ));
            }
            Ok(())
        }
        Action::TransferFromCall { contract, .. } => Err(Denial::never(
            contract,
            format!(
                "transferFrom on {} is never signed (it moves tokens on an allowance)",
                contract.to_checksum(None)
            ),
        )),
        Action::ContractCall {
            contract, selector, ..
        } => {
            let sel = format!("0x{}", hex::encode(selector));
            if registry::is_token_anywhere(contract) {
                return Err(Denial::never(
                    contract,
                    format!(
                        "{} is a registry token: it takes only a transfer to a payee and an \
                         approve of 0, never selector {sel}",
                        contract.to_checksum(None)
                    ),
                ));
            }
            let addr = contract.to_checksum(None);
            if list
                .contract(&addr, chain.key)
                .is_some_and(|e| e.allows_selector(&sel))
            {
                return Ok(());
            }
            Err(match list.by_address(&addr) {
                Some(e) if e.kind == EntryKind::Contract => {
                    let mut missing = Vec::new();
                    if !e.covers_chain(chain.key) {
                        missing.push(format!("chain {}", chain.key));
                    }
                    if !e.allows_selector(&sel) {
                        missing.push(format!("selector {sel}"));
                    }
                    Denial {
                        counterparty: *contract,
                        why: format!(
                            "contract '{}' ({addr}) does not list {}",
                            e.nickname,
                            missing.join(" or ")
                        ),
                        fix: Fix::Widen,
                    }
                }
                Some(e) => Denial::never(
                    contract,
                    format!(
                        "{addr} is the payee '{}', not a contract entry; a call needs a contract \
                         entry",
                        e.nickname
                    ),
                ),
                None => Denial {
                    counterparty: *contract,
                    why: format!("{addr} is not a contract entry on {}", chain.key),
                    fix: Fix::Add(format!(
                        "zwallet whitelist add <nick> {addr} --chains {} --contract --selectors \
                         {sel}",
                        chain.key
                    )),
                },
            })
        }
    }
}

/// `to` must be an address entry that covers `chain`.
fn payee(list: &Whitelist, chain: &Chain, to: &Address) -> std::result::Result<(), Denial> {
    let addr = to.to_checksum(None);
    if list.payee(&addr, chain.key).is_some() {
        return Ok(());
    }
    Err(match list.by_address(&addr) {
        Some(e) if e.kind == EntryKind::Address => Denial {
            counterparty: *to,
            why: format!(
                "payee '{}' ({addr}) is listed for {} only, not {}",
                e.nickname,
                e.chains.join(","),
                chain.key
            ),
            fix: Fix::Widen,
        },
        Some(e) => Denial::never(
            to,
            format!(
                "{addr} is the contract '{}', which takes calls with its selectors, never a \
                 payment",
                e.nickname
            ),
        ),
        None => Denial {
            counterparty: *to,
            why: format!("{addr} is not in the whitelist as a payee on {}", chain.key),
            fix: Fix::Add(format!(
                "zwallet whitelist add <nick> {addr} --chains {}",
                chain.key
            )),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::evm::tx::{self, EvmFees, UnsignedEvmTx};
    use crate::chains::find;
    use crate::core::amount::parse_units;
    use crate::core::error::ErrorCode;
    use crate::core::whitelist::{Entry, Policy};
    use alloy::primitives::{U128, U256};
    use std::collections::BTreeMap;

    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const ALICE: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
    const BOB: &str = "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359";
    const CAROL: &str = "0x4444444444444444444444444444444444444444";
    const ROUTER: &str = "0x2222222222222222222222222222222222222222";
    const SPENDER: &str = "0x3333333333333333333333333333333333333333";
    const USDC_ETH: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    const USDT_ETH: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
    const USDC_BASE: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    const WETH_BASE: &str = "0x4200000000000000000000000000000000000006";
    const SEL: &str = "0x12345678";
    const DEPOSIT: &str = "0xd0e30db0";
    const GWEI: u64 = 1_000_000_000;

    fn a(s: &str) -> Address {
        s.parse().unwrap()
    }

    fn signer() -> Address {
        a(SIGNER)
    }

    /// alice: payee on base; bob: payee everywhere; router: contract on base with [`SEL`].
    fn list() -> Whitelist {
        let mut l = Whitelist::empty(&registry::WHITELIST_RULES);
        l.generation = 4;
        l.entries = vec![
            Entry::new_address("alice", ALICE, &["base"]),
            Entry::new_address("bob", BOB, &["*"]),
            Entry::new_contract("router", ROUTER, &["base"], &[SEL]),
        ];
        l.validate(&registry::WHITELIST_RULES).unwrap();
        l
    }

    /// What the daemon decodes from the signing payload of this transaction (EIP-1559, 1 wei tip).
    fn decoded(
        chain: &str,
        to: &str,
        value: u64,
        input: &str,
        gas: u64,
        max_fee: u64,
    ) -> DecodedUnsigned {
        let chain_id = crate::chains::evm::config(find(chain).unwrap()).chain_id;
        let t = UnsignedEvmTx {
            chain_id,
            nonce: 3,
            gas_limit: gas,
            fees: EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(max_fee),
                max_priority_fee_per_gas: U128::from(1u64),
            },
            to: a(to),
            value: U256::from(value),
            input: crate::chains::evm::abi::parse_calldata(input).unwrap(),
        };
        tx::decode_unsigned(&tx::encoded_for_signing(&t)).unwrap()
    }

    /// A cheap transaction: 21000 gas at 1 gwei.
    fn cheap(chain: &str, to: &str, value: u64, input: &str) -> DecodedUnsigned {
        decoded(chain, to, value, input, 21_000, GWEI)
    }

    fn word(addr: &str) -> String {
        format!(
            "{:0>64}",
            addr.trim_start_matches("0x").to_ascii_lowercase()
        )
    }

    fn uint(n: u64) -> String {
        format!("{n:064x}")
    }

    fn transfer_data(to: &str, amount: u64) -> String {
        format!("0xa9059cbb{}{}", word(to), uint(amount))
    }

    fn approve_data(spender: &str, amount: u64) -> String {
        format!("0x095ea7b3{}{}", word(spender), uint(amount))
    }

    fn transfer_from_data(from: &str, to: &str, amount: u64) -> String {
        format!("0x23b872dd{}{}{}", word(from), word(to), uint(amount))
    }

    fn check(view: &ListView, chain: &str, d: &DecodedUnsigned) -> Result<()> {
        check_tx(view, find(chain).unwrap(), &signer(), d)
    }

    /// Asserts a refusal with `code` about `counterparty` on `chain`, carrying exactly `command`
    /// (in the details and in the message). Returns the message.
    fn refused(
        r: Result<()>,
        code: ErrorCode,
        counterparty: &str,
        chain: &str,
        command: Option<&str>,
    ) -> String {
        let e = r.unwrap_err();
        assert_eq!(e.code, code, "{}", e.message);
        assert_eq!(e.details["counterparty"], counterparty, "{e:?}");
        assert_eq!(e.details["chain"], chain, "{e:?}");
        match command {
            Some(c) => {
                assert_eq!(e.details["command"], c, "{e:?}");
                assert!(e.message.contains(&format!("`{c}`")), "{}", e.message);
            }
            None => assert!(!e.details.contains_key("command"), "{e:?}"),
        }
        assert!(e.message.starts_with("refusing to sign: "), "{}", e.message);
        e.message
    }

    fn add_cmd(addr: &str, chain: &str) -> String {
        format!("zwallet whitelist add <nick> {addr} --chains {chain}")
    }

    #[test]
    fn native_transfers_go_to_payees_on_their_chains_or_to_the_signer() {
        let l = list();
        let v = ListView::Usable(&l);
        check(&v, "base", &cheap("base", ALICE, 5, "0x")).unwrap();
        check(&v, "base", &cheap("base", BOB, 5, "0x")).unwrap();
        check(&v, "ethereum", &cheap("ethereum", BOB, 5, "0x")).unwrap();
        check(&v, "ethereum", &cheap("ethereum", SIGNER, 5, "0x")).unwrap();
        // Lower-case and checksummed entries are the same address.
        check(&v, "base", &cheap("base", &ALICE.to_lowercase(), 5, "0x")).unwrap();
        // alice is listed for base only: the message names her entry and the chain it lacks,
        // and suggests no command (changing an entry is a remove and an add, the human's call).
        let msg = refused(
            check(&v, "ethereum", &cheap("ethereum", ALICE, 5, "0x")),
            ErrorCode::WhitelistDenied,
            ALICE,
            "ethereum",
            None,
        );
        assert!(msg.contains("'alice'"), "{msg}");
        assert!(msg.contains("listed for base only, not ethereum"), "{msg}");
        assert!(!msg.contains("zwallet whitelist"), "{msg}");
        refused(
            check(&v, "base", &cheap("base", CAROL, 5, "0x")),
            ErrorCode::WhitelistDenied,
            CAROL,
            "base",
            Some(&add_cmd(CAROL, "base")),
        );
        // A contract entry is no payee.
        refused(
            check(&v, "base", &cheap("base", ROUTER, 5, "0x")),
            ErrorCode::WhitelistDenied,
            ROUTER,
            "base",
            None,
        );
    }

    /// A legacy (EIP-155) transaction on `chain`.
    fn legacy(chain: &str, to: &str, value: u64, gas: u64, price: u64) -> DecodedUnsigned {
        let t = UnsignedEvmTx {
            chain_id: crate::chains::evm::config(find(chain).unwrap()).chain_id,
            nonce: 0,
            gas_limit: gas,
            fees: EvmFees::Legacy {
                gas_price: U128::from(price),
            },
            to: a(to),
            value: U256::from(value),
            input: Default::default(),
        };
        tx::decode_unsigned(&tx::encoded_for_signing(&t)).unwrap()
    }

    /// The compiled-in default fee cap of `chain`, in wei.
    fn default_cap(chain: &str) -> u64 {
        let (_, wei) = registry::DEFAULT_FEE_CAPS_WEI
            .iter()
            .find(|(k, _)| *k == chain)
            .unwrap();
        wei.parse().unwrap()
    }

    #[test]
    fn with_no_list_yet_refusals_name_the_edit_that_creates_one() {
        let v = ListView::NoList;
        check(&v, "ethereum", &cheap("ethereum", SIGNER, u64::MAX, "0x")).unwrap();
        let msg = refused(
            check(&v, "ethereum", &cheap("ethereum", BOB, 1, "0x")),
            ErrorCode::WhitelistInvalid,
            BOB,
            "ethereum",
            Some(&add_cmd(BOB, "ethereum")),
        );
        assert!(msg.contains(NO_WHITELIST), "{msg}");
        // Contract calls name the contract entry that would allow them.
        let cmd = format!(
            "zwallet whitelist add <nick> {ROUTER} --chains base --contract --selectors {SEL}"
        );
        refused(
            check(&v, "base", &cheap("base", ROUTER, 0, SEL)),
            ErrorCode::WhitelistInvalid,
            ROUTER,
            "base",
            Some(&cmd),
        );
        // Even a revoke, which a usable list needs no entry for.
        let revoke = cheap("ethereum", USDC_ETH, 0, &approve_data(SPENDER, 0));
        refused(
            check(&v, "ethereum", &revoke),
            ErrorCode::WhitelistInvalid,
            USDC_ETH,
            "ethereum",
            None,
        );
        // A token transfer to the signer is no native self-transfer.
        let token_self = cheap("ethereum", USDC_ETH, 0, &transfer_data(SIGNER, 1));
        refused(
            check(&v, "ethereum", &token_self),
            ErrorCode::WhitelistInvalid,
            SIGNER,
            "ethereum",
            Some(&add_cmd(SIGNER, "ethereum")),
        );
    }

    #[test]
    fn a_list_that_cannot_be_trusted_names_no_command_and_points_to_verify() {
        // Tampered with or unreadable: suggesting an entry for the address in the request would
        // hand whoever broke the list the address it picked. The human checks the list instead.
        let reason = "whitelist.json: whitelist signature does not verify";
        let v = ListView::Unusable(reason.into());
        let revoke = approve_data(SPENDER, 0);
        let token_self = transfer_data(SIGNER, 1);
        let cases = [
            // Not even a cheap transfer to the key's own address.
            ("ethereum", cheap("ethereum", SIGNER, 0, "0x"), SIGNER),
            ("ethereum", cheap("ethereum", BOB, 1, "0x"), BOB),
            ("base", cheap("base", ROUTER, 0, SEL), ROUTER),
            (
                "ethereum",
                cheap("ethereum", USDC_ETH, 0, &revoke),
                USDC_ETH,
            ),
            (
                "ethereum",
                cheap("ethereum", USDC_ETH, 0, &token_self),
                SIGNER,
            ),
        ];
        for (chain, d, counterparty) in cases {
            let msg = refused(
                check(&v, chain, &d),
                ErrorCode::WhitelistInvalid,
                counterparty,
                chain,
                None,
            );
            assert!(msg.contains(reason), "{msg}");
            assert!(msg.contains(&format!("`{VERIFY_COMMAND}`")), "{msg}");
            assert!(
                msg.contains("not even a transfer to this key's own"),
                "{msg}"
            );
            assert!(!msg.contains("whitelist add"), "{msg}");
        }
    }

    #[test]
    fn breaking_the_list_never_relaxes_its_signed_fee_cap() {
        // P-1: a verified list caps base at L below the built-in default D. A self-transfer with
        // L < fee <= D is refused under it, and must stay refused once the list is broken.
        let l = {
            let mut l = list();
            l.policy.set_max_fee_wei("base", U256::from(21_000 * GWEI));
            l
        };
        let price = default_cap("base") / 21_000;
        let d = decoded("base", SIGNER, 1, "0x", 21_000, price);
        assert!(U256::from(21_000 * price) > U256::from(21_000 * GWEI));
        let e = check(&ListView::Usable(&l), "base", &d).unwrap_err();
        assert_eq!(e.code, ErrorCode::FeeCapExceeded, "{}", e.message);
        let broken = ListView::Unusable("whitelist.json: not a whitelist file".into());
        let msg = refused(
            check(&broken, "base", &d),
            ErrorCode::WhitelistInvalid,
            SIGNER,
            "base",
            None,
        );
        assert!(msg.contains(&format!("`{VERIFY_COMMAND}`")), "{msg}");
    }

    /// A chain zwallet has no built-in fee cap for (every real chain has one).
    const NO_DEFAULT: Chain = Chain {
        key: "nodefault",
        name: "No default",
        symbol: "ETH",
        decimals: 18,
        family: crate::chains::Family::Evm(crate::chains::evm::EvmChain {
            chain_id: 1,
            default_rpc: "",
            explorer: "",
            op_stack: false,
        }),
    };

    #[test]
    fn self_transfers_are_always_under_a_fee_cap() {
        // No list yet: the compiled-in default cap applies to the key's own address.
        for c in crate::chains::CHAINS {
            let gas = 100_000;
            let at = default_cap(c.key) / gas;
            let v = ListView::NoList;
            check(&v, c.key, &decoded(c.key, SIGNER, u64::MAX, "0x", gas, at)).unwrap();
            check(&v, c.key, &legacy(c.key, SIGNER, u64::MAX, gas, at)).unwrap();
            for over in [
                decoded(c.key, SIGNER, 0, "0x", gas, at + 1),
                legacy(c.key, SIGNER, 0, gas, at + 1),
            ] {
                let e = check(&v, c.key, &over).unwrap_err();
                assert_eq!(
                    e.code,
                    ErrorCode::FeeCapExceeded,
                    "{}: {}",
                    c.key,
                    e.message
                );
                assert_eq!(e.details["capWei"], default_cap(c.key).to_string());
                assert_eq!(e.details["counterparty"], SIGNER);
                assert!(e.message.contains("built-in"), "{}", e.message);
                // The first fee-cap edit creates the list with that cap.
                assert!(
                    e.details["command"]
                        .as_str()
                        .unwrap()
                        .starts_with(&format!("zwallet whitelist fee-cap {} ", c.key)),
                    "{e:?}"
                );
            }
        }
        // No list and no built-in cap for the chain: refused, naming the edit that sets one.
        let e = check_tx(
            &ListView::NoList,
            &NO_DEFAULT,
            &signer(),
            &cheap("ethereum", SIGNER, 0, "0x"),
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::FeeCapExceeded, "{}", e.message);
        assert!(!e.details.contains_key("capWei"), "{e:?}");
        assert!(
            e.details["command"]
                .as_str()
                .unwrap()
                .starts_with("zwallet whitelist fee-cap nodefault "),
            "{e:?}"
        );
        // A usable list's own cap, here below the default.
        let mut l = list();
        l.policy.set_max_fee_wei("base", U256::from(21_000 * GWEI));
        let v = ListView::Usable(&l);
        check(&v, "base", &cheap("base", SIGNER, 1, "0x")).unwrap();
        let over = decoded("base", SIGNER, 1, "0x", 21_000, GWEI + 1);
        let e = check(&v, "base", &over).unwrap_err();
        assert_eq!(e.code, ErrorCode::FeeCapExceeded);
        assert_eq!(e.details["capWei"], (21_000 * GWEI).to_string());
    }

    /// A transaction with the widest fee operands there are: u64::MAX gas at u128::MAX wei.
    fn widest(chain_id: u64, to: &str, input: &str, eip1559: bool) -> DecodedUnsigned {
        let fees = if eip1559 {
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::MAX,
                max_priority_fee_per_gas: U128::MAX,
            }
        } else {
            EvmFees::Legacy {
                gas_price: U128::MAX,
            }
        };
        let t = UnsignedEvmTx {
            chain_id,
            nonce: u64::MAX - 1,
            gas_limit: u64::MAX,
            fees,
            to: a(to),
            value: U256::ZERO,
            input: crate::chains::evm::abi::parse_calldata(input).unwrap(),
        };
        tx::decode_unsigned(&tx::encoded_for_signing(&t)).unwrap()
    }

    #[test]
    fn a_broken_list_is_never_more_permissive_than_a_verified_default_list() {
        // Over every action kind, target, fee shape and chain (plus one with no built-in cap):
        // an untrusted list allows nothing at all, and no list yet allows nothing a verified list
        // with no entries and the default policy refuses.
        let mut verified = Whitelist::empty(&registry::WHITELIST_RULES);
        verified.generation = 1;
        verified.validate(&registry::WHITELIST_RULES).unwrap();
        let untrusted = ListView::Unusable("whitelist.json: does not verify".into());
        let (mut cases, mut allowed) = (0, 0);
        let mut chains: Vec<&Chain> = crate::chains::CHAINS.iter().collect();
        chains.push(&NO_DEFAULT);
        for c in chains {
            let gas = 100_000;
            let cap = default_fee_cap(c.key).map_or(GWEI, |d| d.to::<u64>() / gas);
            let id = crate::chains::evm::config(c).chain_id;
            let mut targets = vec![SIGNER, BOB, ROUTER, USDC_ETH, WETH_BASE];
            targets.extend(
                registry::TOKENS
                    .iter()
                    .filter(|t| t.chain_id == id)
                    .map(|t| t.address),
            );
            let mut calls = vec!["0x".to_string(), SEL.into(), DEPOSIT.into()];
            for t in &targets {
                calls.push(transfer_data(t, 1));
                calls.push(approve_data(t, 0));
                calls.push(approve_data(t, 1));
                calls.push(transfer_from_data(t, SIGNER, 1));
            }
            for to in &targets {
                for data in &calls {
                    let value = if data.len() > 10 { 0 } else { 5 };
                    let mut txs = Vec::new();
                    for price in [1, cap, cap + 1, cap * 10] {
                        let t = UnsignedEvmTx {
                            chain_id: id,
                            nonce: 3,
                            gas_limit: gas,
                            fees: EvmFees::Eip1559 {
                                max_fee_per_gas: U128::from(price),
                                max_priority_fee_per_gas: U128::from(1u64),
                            },
                            to: a(to),
                            value: U256::from(value),
                            input: crate::chains::evm::abi::parse_calldata(data).unwrap(),
                        };
                        txs.push(tx::decode_unsigned(&tx::encoded_for_signing(&t)).unwrap());
                        if data == "0x" {
                            let t = UnsignedEvmTx {
                                fees: EvmFees::Legacy {
                                    gas_price: U128::from(price),
                                },
                                ..t
                            };
                            txs.push(tx::decode_unsigned(&tx::encoded_for_signing(&t)).unwrap());
                        }
                    }
                    txs.push(widest(id, to, data, true));
                    if data == "0x" {
                        txs.push(widest(id, to, data, false));
                    }
                    for d in txs {
                        cases += 1;
                        let r = check_tx(&untrusted, c, &signer(), &d);
                        let e = r.expect_err("an untrusted list allowed a signature");
                        assert_eq!(e.code, ErrorCode::WhitelistInvalid, "{}", e.message);
                        assert!(!e.details.contains_key("command"), "{e:?}");
                        let strict = check_tx(&ListView::Usable(&verified), c, &signer(), &d);
                        if check_tx(&ListView::NoList, c, &signer(), &d).is_ok() {
                            allowed += 1;
                            assert!(
                                strict.is_ok(),
                                "NoList allows on {} what a verified default list refuses: {:?}",
                                c.key,
                                d.to_json()
                            );
                        }
                    }
                }
            }
        }
        // Not vacuous: some self-transfers pass with no list yet, most of the corpus does not.
        assert!(allowed > 0 && allowed < cases / 10, "{allowed} of {cases}");
    }

    #[test]
    fn token_transfers_need_a_registry_token_on_this_chain_and_a_payee() {
        let l = list();
        let v = ListView::Usable(&l);
        let send = |token: &str, to: &str| cheap("ethereum", token, 0, &transfer_data(to, 7));
        check(&v, "ethereum", &send(USDC_ETH, BOB)).unwrap();
        check(&v, "ethereum", &send(USDT_ETH, BOB)).unwrap();
        refused(
            check(&v, "ethereum", &send(USDC_ETH, CAROL)),
            ErrorCode::WhitelistDenied,
            CAROL,
            "ethereum",
            Some(&add_cmd(CAROL, "ethereum")),
        );
        // Not a token zwallet knows, or the right address on the wrong chain.
        for token in [ROUTER, USDC_BASE] {
            let msg = refused(
                check(&v, "ethereum", &send(token, BOB)),
                ErrorCode::WhitelistDenied,
                token,
                "ethereum",
                None,
            );
            assert!(msg.contains("registry token"), "{msg}");
        }
        // To a token contract (any chain's), even one listed as a payee.
        let mut l = list();
        l.add(Entry::new_address("usdt", USDT_ETH, &["*"])).unwrap();
        let v = ListView::Usable(&l);
        for to in [USDT_ETH, USDC_ETH, USDC_BASE] {
            refused(
                check(&v, "ethereum", &send(USDC_ETH, to)),
                ErrorCode::WhitelistDenied,
                to,
                "ethereum",
                None,
            );
        }
        // A native payment to that payee is fine (the list allows the address).
        check(&v, "ethereum", &cheap("ethereum", USDT_ETH, 1, "0x")).unwrap();
    }

    #[test]
    fn approvals_are_revoke_only_on_registry_tokens_and_transfer_from_never() {
        let l = list();
        let v = ListView::Usable(&l);
        check(
            &v,
            "ethereum",
            &cheap("ethereum", USDC_ETH, 0, &approve_data(CAROL, 0)),
        )
        .unwrap();
        check(
            &v,
            "base",
            &cheap("base", WETH_BASE, 0, &approve_data(SPENDER, 0)),
        )
        .unwrap();
        let msg = refused(
            check(
                &v,
                "ethereum",
                &cheap("ethereum", USDC_ETH, 0, &approve_data(SPENDER, 1)),
            ),
            ErrorCode::WhitelistDenied,
            SPENDER,
            "ethereum",
            None,
        );
        assert!(msg.contains("revoke-only"), "{msg}");
        // A spender on the list does not help.
        refused(
            check(
                &v,
                "ethereum",
                &cheap("ethereum", USDC_ETH, 0, &approve_data(BOB, 1)),
            ),
            ErrorCode::WhitelistDenied,
            BOB,
            "ethereum",
            None,
        );
        // Outside the registry, not even a revoke.
        refused(
            check(
                &v,
                "base",
                &cheap("base", ROUTER, 0, &approve_data(SPENDER, 0)),
            ),
            ErrorCode::WhitelistDenied,
            ROUTER,
            "base",
            None,
        );
        let msg = refused(
            check(
                &v,
                "ethereum",
                &cheap("ethereum", USDC_ETH, 0, &transfer_from_data(BOB, SIGNER, 1)),
            ),
            ErrorCode::WhitelistDenied,
            USDC_ETH,
            "ethereum",
            None,
        );
        assert!(msg.contains("transferFrom"), "{msg}");
    }

    #[test]
    fn contract_calls_need_a_contract_entry_on_this_chain_with_the_selector() {
        let l = list();
        let v = ListView::Usable(&l);
        check(&v, "base", &cheap("base", ROUTER, 0, SEL)).unwrap();
        // Native value rides along only on a listed call.
        check(
            &v,
            "base",
            &cheap("base", ROUTER, 10u64.pow(15), &format!("{SEL}00")),
        )
        .unwrap();
        // A listed contract without this selector, or this chain: the entry and what it lacks.
        let msg = refused(
            check(&v, "base", &cheap("base", ROUTER, 0, DEPOSIT)),
            ErrorCode::WhitelistDenied,
            ROUTER,
            "base",
            None,
        );
        assert!(
            msg.contains(&format!(
                "contract 'router' ({ROUTER}) does not list selector {DEPOSIT}"
            )),
            "{msg}"
        );
        let msg = refused(
            check(&v, "ethereum", &cheap("ethereum", ROUTER, 0, SEL)),
            ErrorCode::WhitelistDenied,
            ROUTER,
            "ethereum",
            None,
        );
        assert!(msg.contains("does not list chain ethereum"), "{msg}");
        assert!(!msg.contains("zwallet whitelist"), "{msg}");
        let cmd = format!(
            "zwallet whitelist add <nick> {CAROL} --chains base --contract --selectors {DEPOSIT}"
        );
        refused(
            check(&v, "base", &cheap("base", CAROL, 1, DEPOSIT)),
            ErrorCode::WhitelistDenied,
            CAROL,
            "base",
            Some(&cmd),
        );
        // A payee is no contract entry.
        refused(
            check(&v, "base", &cheap("base", ALICE, 0, SEL)),
            ErrorCode::WhitelistDenied,
            ALICE,
            "base",
            None,
        );
    }

    #[test]
    fn a_call_on_a_registry_token_is_denied_whatever_the_list_says() {
        // A list no edit would sign (a contract entry at registry tokens), as if it verified:
        // the policy does not rely on the list's validation for this.
        let mut l = list();
        l.entries.push(Entry::new_contract(
            "weth",
            WETH_BASE,
            &["base"],
            &[DEPOSIT],
        ));
        l.entries
            .push(Entry::new_contract("usdc", USDC_ETH, &["base"], &[DEPOSIT]));
        let v = ListView::Usable(&l);
        for token in [WETH_BASE, USDC_ETH] {
            let msg = refused(
                check(&v, "base", &cheap("base", token, 0, DEPOSIT)),
                ErrorCode::WhitelistDenied,
                token,
                "base",
                None,
            );
            assert!(msg.contains("registry token"), "{msg}");
        }
    }

    #[test]
    fn the_fee_limit_must_be_within_the_lists_cap_for_the_chain() {
        let mut l = list();
        let cap = 300_000 * GWEI; // 0.0003 ETH
        l.policy.set_max_fee_wei("base", U256::from(cap));
        let v = ListView::Usable(&l);
        // Exactly at the cap: allowed (gasLimit x maxFeePerGas, not the tip).
        check(
            &v,
            "base",
            &decoded("base", BOB, 1, "0x", 100_000, 3 * GWEI),
        )
        .unwrap();
        let over = decoded("base", BOB, 1, "0x", 100_000, 3 * GWEI + 1);
        let e = check(&v, "base", &over).unwrap_err();
        let fee = U256::from(100_000u64) * U256::from(3 * GWEI + 1);
        assert_eq!(e.details["feeWei"], fee.to_string());
        assert_eq!(e.details["capWei"], cap.to_string());
        let amount = "0.0003000000001";
        assert_eq!(parse_units(amount, 18).unwrap(), fee);
        let cmd = format!("zwallet whitelist fee-cap base {amount}");
        refused(Err(e), ErrorCode::FeeCapExceeded, BOB, "base", Some(&cmd));
        // Before the action table: an unlisted payee over the cap is a fee refusal.
        let unlisted = decoded("base", CAROL, 1, "0x", 100_000, 3 * GWEI + 1);
        assert_eq!(
            check(&v, "base", &unlisted).unwrap_err().code,
            ErrorCode::FeeCapExceeded
        );
        // The signer's own address too, while a list is usable.
        let own = decoded("base", SIGNER, 1, "0x", 100_000, 3 * GWEI + 1);
        assert_eq!(
            check(&v, "base", &own).unwrap_err().code,
            ErrorCode::FeeCapExceeded
        );
        // A legacy transaction's gasPrice is its price.
        let legacy = |price: u64| {
            let t = UnsignedEvmTx {
                chain_id: 8453,
                nonce: 0,
                gas_limit: 100_000,
                fees: EvmFees::Legacy {
                    gas_price: U128::from(price),
                },
                to: a(BOB),
                value: U256::from(1u64),
                input: Default::default(),
            };
            tx::decode_unsigned(&tx::encoded_for_signing(&t)).unwrap()
        };
        check(&v, "base", &legacy(3 * GWEI)).unwrap();
        assert_eq!(
            check(&v, "base", &legacy(3 * GWEI + 1)).unwrap_err().code,
            ErrorCode::FeeCapExceeded
        );
    }

    #[test]
    fn a_missing_or_malformed_fee_cap_refuses_every_send_on_that_chain() {
        for bad in [None, Some("0500"), Some("5e14"), Some("")] {
            let mut l = list();
            match bad {
                None => {
                    l.policy.max_fee_wei.remove("base");
                }
                Some(s) => {
                    l.policy.max_fee_wei.insert("base".into(), s.into());
                }
            }
            let v = ListView::Usable(&l);
            let e = check(&v, "base", &cheap("base", BOB, 1, "0x")).unwrap_err();
            assert_eq!(e.code, ErrorCode::FeeCapExceeded, "{bad:?}");
            assert!(e.message.contains("no fee cap for base"), "{}", e.message);
            assert_eq!(
                e.details["command"],
                "zwallet whitelist fee-cap base 0.000021"
            );
            assert!(!e.details.contains_key("capWei"), "{e:?}");
            // Including the signer's own address: a usable list always applies its cap.
            let own = check(&v, "base", &cheap("base", SIGNER, 1, "0x")).unwrap_err();
            assert_eq!(own.code, ErrorCode::FeeCapExceeded);
            // Other chains keep their caps.
            check(&v, "ethereum", &cheap("ethereum", BOB, 1, "0x")).unwrap();
        }
        // A policy with no caps at all.
        let mut l = list();
        l.policy = Policy {
            max_fee_wei: BTreeMap::new(),
            allow_message_signing: false,
        };
        let e = check(
            &ListView::Usable(&l),
            "polygon",
            &cheap("polygon", BOB, 1, "0x"),
        );
        assert_eq!(e.unwrap_err().code, ErrorCode::FeeCapExceeded);
    }

    #[test]
    fn w10_messages_need_a_usable_list_that_allows_them() {
        let mut l = list();
        let e = check_message(&ListView::Usable(&l)).unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistDenied);
        assert_eq!(e.details["command"], MESSAGE_SIGNING_COMMAND);
        assert!(e.message.contains(MESSAGE_SIGNING_COMMAND), "{}", e.message);
        l.policy.allow_message_signing = true;
        check_message(&ListView::Usable(&l)).unwrap();
        // No list yet: turning message signing on creates it.
        let e = check_message(&ListView::NoList).unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistInvalid);
        assert!(e.message.contains(NO_WHITELIST), "{}", e.message);
        assert_eq!(e.details["command"], MESSAGE_SIGNING_COMMAND);
        // A list that cannot be trusted: checked first, no edit suggested.
        let reason = "whitelist.json: whitelist signature does not verify";
        let e = check_message(&ListView::Unusable(reason.into())).unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistInvalid);
        assert!(e.message.contains(reason), "{}", e.message);
        assert!(e.message.contains(VERIFY_COMMAND), "{}", e.message);
        assert!(!e.details.contains_key("command"), "{e:?}");
    }
}

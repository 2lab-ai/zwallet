//! Shared BIP39 / SLIP-0010 Ed25519 derivation and native-transfer policy.
//!
//! This is key-side code: no RPC or filesystem access. The family decodes its unsigned bytes
//! and validates its supported transaction shape before calling [`check_native`]. All paths
//! are fully hardened. TON uses the multichain BIP39 scheme, not TON-native mnemonic entropy.
//! Locally held seed, chain-code and derivation buffers are wiped. The underlying BIP39/HMAC
//! implementations may retain allocator/stack copies, as documented for the existing seed path.

use crate::chains::{Chain, Family, policy::ListView};
use crate::core::{
    amount::format_units,
    error::{ErrorCode, Result, WalletError},
    seed::Seed,
    store::KEY_INDEX_LIMIT,
};
use alloy::{
    hex,
    primitives::U256,
    signers::local::coins_bip39::{English, Mnemonic},
};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

/// The family path is explicit recovery metadata; all components are hardened.
pub fn derivation_path(coin: u32, index: u32, change: Option<u32>) -> String {
    let mut path = format!("m/44'/{coin}'/{index}'");
    if let Some(change) = change {
        path.push_str(&format!("/{change}'"));
    }
    path
}

/// Derive a signing key from the wallet's existing BIP39 phrase with the empty passphrase.
pub fn signer_at(seed: &Seed, coin: u32, index: u32, change: Option<u32>) -> Result<SigningKey> {
    if coin >= KEY_INDEX_LIMIT
        || index >= KEY_INDEX_LIMIT
        || change.is_some_and(|v| v >= KEY_INDEX_LIMIT)
    {
        return Err(WalletError::validation(
            "Ed25519 derivation components must be below 2^31",
        ));
    }
    let mnemonic = Mnemonic::<English>::new_from_phrase(seed.phrase())
        .map_err(|_| WalletError::validation("cannot derive Ed25519 key from the wallet seed"))?;
    let seed_bytes =
        Zeroizing::new(mnemonic.to_seed(None).map_err(|_| {
            WalletError::validation("cannot derive Ed25519 key from the wallet seed")
        })?);
    let mut path = vec![44, coin, index];
    path.extend(change);
    let node = derive_node(seed_bytes.as_ref(), &path)?;
    let mut secret = Zeroizing::new([0u8; 32]);
    secret.copy_from_slice(&node[..32]);
    Ok(SigningKey::from_bytes(&secret))
}

fn hmac512(key: &[u8], data: &[u8]) -> Result<Zeroizing<[u8; 64]>> {
    let mut mac = Hmac::<Sha512>::new_from_slice(key)
        .map_err(|_| WalletError::validation("cannot initialize Ed25519 derivation"))?;
    mac.update(data);
    Ok(Zeroizing::new(mac.finalize().into_bytes().into()))
}

/// Returns key || chain code. `path` uses indices before the hardened bit is added.
fn derive_node(seed: &[u8], path: &[u32]) -> Result<Zeroizing<[u8; 64]>> {
    if path.iter().any(|index| *index >= KEY_INDEX_LIMIT) {
        return Err(WalletError::validation(
            "Ed25519 derivation components must be below 2^31",
        ));
    }
    let mut node = hmac512(b"ed25519 seed", seed)?;
    for index in path {
        let mut data = Zeroizing::new([0u8; 37]);
        data[1..33].copy_from_slice(&node[..32]);
        data[33..].copy_from_slice(&(index | KEY_INDEX_LIMIT).to_be_bytes());
        node = hmac512(&node[32..], data.as_ref())?;
    }
    Ok(node)
}

/// Stable, case-insensitive-safe whitelist representation of a case-sensitive public address.
pub fn policy_address(chain: &Chain, address: &str) -> Result<String> {
    match chain.family {
        Family::Solana(_) => crate::chains::solana::policy_address(address),
        Family::Ton(_) => crate::chains::ton::policy_address(address),
        _ => Err(WalletError::validation(
            "an Ed25519 policy needs Solana or TON",
        )),
    }
}

/// Recover the public user-facing address from a tagged whitelist identity.
pub fn display_policy_address(address: &str) -> Result<String> {
    if crate::core::whitelist::tagged_address_family(address).is_none() {
        return Err(WalletError::validation(
            "invalid Ed25519 whitelist identity",
        ));
    }
    if let Some(body) = address.strip_prefix("solana:") {
        let bytes: [u8; 32] = hex::decode(body)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| WalletError::validation("invalid Solana whitelist identity"))?;
        return crate::chains::solana::normalize_address(&bs58::encode(bytes).into_string());
    }
    if let Some(raw) = address.strip_prefix("ton:") {
        return crate::chains::ton::normalize_address(raw);
    }
    Err(WalletError::validation(
        "invalid Ed25519 whitelist identity",
    ))
}

/// Require an exact-family payee and the signed native amount / fee policy.
/// `fee` is the family's locally decoded fee envelope, not an RPC authority claim. Where a
/// protocol has no encoded fee ceiling (TON), the family documents that limitation explicitly.
pub fn check_native(
    view: &ListView<'_>,
    chain: &Chain,
    signer: &str,
    to: &str,
    amount: U256,
    fee: U256,
) -> Result<()> {
    // Both public identities must be valid even if the policy would otherwise reject early.
    let _signer = policy_address(chain, signer)?;
    let target = policy_address(chain, to)?;
    if amount.is_zero() {
        return Err(WalletError::validation(
            "refusing to sign: native transfer amount must be positive",
        ));
    }
    let denied = |code, why: &str| {
        WalletError::new(code, format!("refusing to sign: {why}"))
            .with_detail("chain", chain.key)
            .with_detail("counterparty", to)
    };
    let list = match view {
        ListView::Usable(list) => *list,
        ListView::NoList => {
            return Err(denied(
                ErrorCode::WhitelistInvalid,
                crate::chains::policy::NO_WHITELIST,
            ));
        }
        ListView::Unusable(_) => {
            return Err(denied(
                ErrorCode::WhitelistInvalid,
                "the whitelist is not trusted; run `zwallet whitelist list --verify`",
            ));
        }
    };
    // Do not revalidate unrelated legacy entries against a newer token registry. Core
    // verification follows the same rule; this signature's exact chain/payee is checked below.
    list.check_format().map_err(|_| denied(ErrorCode::WhitelistInvalid,
        "the whitelist has an invalid family, address or policy shape; run `zwallet whitelist list --verify`"))?;
    if list
        .payee(&target, chain.key, false)
        .is_none_or(|entry| entry.chains != [chain.key])
    {
        return Err(denied(
            ErrorCode::WhitelistDenied,
            "the recipient is not a payee scoped to this exact chain",
        )
        .with_detail(
            "command",
            format!("zwallet whitelist add <nick> {to} --chains {}", chain.key),
        ));
    }
    if let Some(cap) = list
        .policy
        .transaction_limits
        .get(chain.key)
        .and_then(|limits| limits.max_native_wei.as_ref())
    {
        let cap = U256::from_str_radix(cap, 10).map_err(|_| {
            denied(
                ErrorCode::WhitelistInvalid,
                "the native amount cap is invalid",
            )
        })?;
        if amount > cap {
            return Err(denied(
                ErrorCode::WhitelistDenied,
                "the per-transaction native amount cap is exceeded",
            )
            .with_detail("amountRaw", amount.to_string())
            .with_detail("capRaw", cap.to_string()));
        }
    }
    let cap = list.policy.max_fee_wei(chain.key);
    if cap.is_none_or(|cap| fee > cap) {
        let mut error = denied(
            ErrorCode::FeeCapExceeded,
            "the fee envelope exceeds the signed cap or no fee cap is set",
        )
        .with_detail("feeWei", fee.to_string())
        .with_detail(
            "command",
            format!(
                "zwallet whitelist fee-cap {} {}",
                chain.key,
                format_units(fee.max(U256::from(1)), chain.decimals)
            ),
        );
        if let Some(cap) = cap {
            error = error.with_detail("capWei", cap.to_string());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::WHITELIST_RULES;

    #[test]
    fn slip10_public_vectors_match_all_hardened_levels() {
        // Public SLIP-0010 test vector 1, ed25519:
        // https://github.com/satoshilabs/slips/blob/master/slip-0010.md
        let input = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let paths: &[&[u32]] = &[
            &[],
            &[0],
            &[0, 1],
            &[0, 1, 2],
            &[0, 1, 2, 2],
            &[0, 1, 2, 2, 1_000_000_000],
        ];
        let public = [
            "a4b2856bfec510abab89753fac1ac0e1112364e7d250545963f135f2a33188ed",
            "8c8a13df77a28f3445213a0f432fde644acaa215fc72dcdf300d5efaa85d350c",
            "1932a5270f335bed617d5b935c80aedb1a35bd9fc1e31acafd5372c30f5c1187",
            "ae98736566d30ed0e9d2f4486a64bc95740d89c7db33f52121f8ea8f76ff0fc1",
            "8abae2d66361c879b900d204ad2cc4984fa2aa344dd7ddc46007329ac76c429c",
            "3c24da049451555d51a7014a37337aa4e12d41e485abccfa46b47dfb2af54b7a",
        ];
        for (path, expected) in paths.iter().zip(public) {
            let node = derive_node(&input, path).unwrap();
            let key: &[u8; 32] = node[..32].try_into().unwrap();
            assert_eq!(
                hex::encode(SigningKey::from_bytes(key).verifying_key().as_bytes()),
                expected
            );
        }
        assert!(derive_node(&input, &[KEY_INDEX_LIMIT]).is_err());
    }

    #[test]
    fn family_paths_and_account_indices_are_separate() {
        let seed =
            Seed::from_phrase(Zeroizing::new(crate::core::seed::TEST_MNEMONIC.into())).unwrap();
        let sol = signer_at(&seed, 501, 0, Some(0)).unwrap().verifying_key();
        let ton = signer_at(&seed, 607, 0, None).unwrap().verifying_key();
        // Independently generated with Python hashlib.pbkdf2_hmac / hmac and cryptography's
        // Ed25519 public-key conversion from the public TEST_MNEMONIC fixture.
        assert_eq!(
            hex::encode(sol.as_bytes()),
            "0bf32b9f0db09672038fea36139b18f98a5f0149ef4ce0332e44b9a77e83c22d"
        );
        assert_eq!(
            hex::encode(ton.as_bytes()),
            "c7ead976255c7200c7a161675cc41caa71d08e6211f9e110cc699f65507b9358"
        );
        assert_eq!(
            hex::encode(
                signer_at(&seed, 501, 1, Some(0))
                    .unwrap()
                    .verifying_key()
                    .as_bytes()
            ),
            "9245441f4a752ea6328c6d8d2d2afcb36d131db998a47ca04d6d5d8587d0b012"
        );
        assert_eq!(
            hex::encode(
                signer_at(&seed, 607, 1, None)
                    .unwrap()
                    .verifying_key()
                    .as_bytes()
            ),
            "7dc19e075441458842ea21bfcfadb4092a8700deaf51c093155b977a29113517"
        );
        assert_ne!(sol, ton);
        assert_ne!(
            sol,
            signer_at(&seed, 501, 1, Some(0)).unwrap().verifying_key()
        );
        assert_ne!(ton, signer_at(&seed, 607, 1, None).unwrap().verifying_key());
        assert_eq!(derivation_path(501, 0, Some(0)), "m/44'/501'/0'/0'");
        assert_eq!(derivation_path(607, 0, None), "m/44'/607'/0'");
        assert!(signer_at(&seed, 501, KEY_INDEX_LIMIT, Some(0)).is_err());
        assert!(signer_at(&seed, 607, 0, Some(KEY_INDEX_LIMIT)).is_err());
    }
    #[test]
    fn native_policy_requires_exact_payee_and_signed_amount_and_fee_limits() {
        use crate::core::whitelist::{Entry, TransactionLimits, Whitelist};
        for (chain_key, sender, recipient) in [
            (
                "solana",
                bs58::encode([1u8; 32]).into_string(),
                bs58::encode([2u8; 32]).into_string(),
            ),
            (
                "ton",
                format!("0:{}", "11".repeat(32)),
                format!("0:{}", "22".repeat(32)),
            ),
        ] {
            let chain = crate::chains::find(chain_key).unwrap();
            let mut list = Whitelist::empty(&WHITELIST_RULES);
            list.generation = 1;
            list.add(Entry::new_address(
                "bob",
                &policy_address(chain, &recipient).unwrap(),
                &[chain_key],
            ))
            .unwrap();
            list.policy.set_max_fee_wei(chain_key, U256::from(5000));
            list.policy.transaction_limits.insert(
                chain_key.into(),
                TransactionLimits {
                    max_native_wei: Some("100".into()),
                    ..Default::default()
                },
            );
            let check = |view: &ListView, amount: u64, fee: u64| {
                check_native(
                    view,
                    chain,
                    &sender,
                    &recipient,
                    U256::from(amount),
                    U256::from(fee),
                )
            };
            check(&ListView::Usable(&list), 100, 5000).unwrap();
            assert_eq!(
                check(&ListView::Usable(&list), 101, 5000).unwrap_err().code,
                ErrorCode::WhitelistDenied
            );
            assert_eq!(
                check(&ListView::Usable(&list), 100, 5001).unwrap_err().code,
                ErrorCode::FeeCapExceeded
            );
            assert_eq!(
                check(&ListView::NoList, 1, 5000).unwrap_err().code,
                ErrorCode::WhitelistInvalid
            );
            assert_eq!(
                check(&ListView::Unusable("untrusted input".into()), 1, 5000)
                    .unwrap_err()
                    .code,
                ErrorCode::WhitelistInvalid
            );
            let mut wrong = list.clone();
            wrong.entries[0].chains = vec!["*".into()];
            assert!(check(&ListView::Usable(&wrong), 1, 5000).is_err());
            wrong.entries[0].chains = vec![
                if chain_key == "solana" {
                    "ton"
                } else {
                    "solana"
                }
                .into(),
            ];
            assert!(check(&ListView::Usable(&wrong), 1, 5000).is_err());
            wrong.entries.clear();
            assert_eq!(
                check(&ListView::Usable(&wrong), 1, 5000).unwrap_err().code,
                ErrorCode::WhitelistDenied
            );
            list.policy.max_fee_wei.remove(chain_key);
            assert_eq!(
                check(&ListView::Usable(&list), 1, 0).unwrap_err().code,
                ErrorCode::FeeCapExceeded
            );
            assert!(check(&ListView::Usable(&list), 0, 0).is_err());
        }
    }
}

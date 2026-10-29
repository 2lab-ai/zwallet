use super::*;
use crate::chains::{
    FamilyKeys,
    evm::Evm,
    evm::tx::{EvmFees, UnsignedEvmTx},
    tron::{Tron, signer_at},
};
use crate::core::seed::Seed;
use crate::core::whitelist::{Entry, TransactionLimits};
use alloy::primitives::U128;
use serde_json::json;

const PHRASE: &str = "test test test test test test test test test test test junk";
const NOW: u64 = 1_800_000_000_000;
fn owner() -> [u8; 21] {
    address::parse("TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6").unwrap()
}
fn pay_to() -> [u8; 21] {
    address::parse("TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C").unwrap()
}
fn native() -> DecodedTron {
    DecodedTron {
        ref_block_bytes: [0, 1],
        ref_block_hash: [1; 8],
        timestamp: NOW,
        expiration: NOW + 60_000,
        owner: owner(),
        contract: Contract::Transfer {
            to: pay_to(),
            amount_sun: 1_000_000,
        },
        fee_limit: None,
    }
}
fn token() -> DecodedTron {
    let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
    data.extend([0; 12]);
    data.extend(&pay_to()[1..]);
    data.extend(U256::from(1_000_000u64).to_be_bytes::<32>());
    DecodedTron {
        contract: Contract::TriggerSmart {
            contract_address: address::parse(registry::TOKENS[0].address).unwrap(),
            data,
        },
        fee_limit: Some(DEFAULT_CAP),
        ..native()
    }
}
fn listed() -> Whitelist {
    let mut l = Whitelist::empty(&WHITELIST_RULES);
    l.generation = 1;
    l.add(Entry::new_address("bob", &body(&pay_to()), &["tron"]))
        .unwrap();
    l.validate(&WHITELIST_RULES).unwrap();
    l
}
fn check(l: &Whitelist, t: &DecodedTron) -> Result<()> {
    check_tx(&ListView::Usable(l), owner(), t, NOW)
}

#[test]
fn native_and_exact_registry_transfer_obey_the_explicit_tron_payee() {
    let l = listed();
    check(&l, &native()).unwrap();
    check(&l, &token()).unwrap();
    for chains in [&["*"][..], &["base"][..], &["tron", "base"][..]] {
        let mut l = listed();
        l.entries[0].chains = chains.iter().map(|s| s.to_string()).collect();
        for t in [native(), token()] {
            let e = check(&l, &t).unwrap_err();
            assert_eq!(e.code, ErrorCode::WhitelistDenied);
            assert!(!e.details.contains_key("command"));
        }
    }
}

#[test]
fn no_list_unusable_list_and_self_transfer_never_sign() {
    for t in [native(), token()] {
        let e = check_tx(&ListView::NoList, owner(), &t, NOW).unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistInvalid);
        assert_eq!(
            e.details["command"],
            format!(
                "zwallet whitelist add <nick> {} --chains tron",
                address::encode(&pay_to())
            )
        );
        let e = check_tx(
            &ListView::Unusable("bad signature".into()),
            owner(),
            &t,
            NOW,
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistInvalid);
        assert!(!e.details.contains_key("command"));
        assert!(e.message.contains(VERIFY_COMMAND));
    }
    let mut t = native();
    t.contract = Contract::Transfer {
        to: owner(),
        amount_sun: 1,
    };
    let e = check(&listed(), &t).unwrap_err();
    assert_eq!(e.code, ErrorCode::WhitelistDenied);
    assert!(!e.details.contains_key("command"));
}

#[test]
fn owner_and_clock_are_independently_bounded() {
    let mut t = native();
    t.owner = pay_to();
    assert_eq!(
        check(&listed(), &t).unwrap_err().code,
        ErrorCode::ValidationError
    );
    for (timestamp, expiration) in [
        (NOW, NOW),
        (NOW, NOW - 1),
        (NOW, NOW + 300_001),
        (NOW + 1, NOW + 300_001),
        (u64::MAX - 1, u64::MAX),
    ] {
        let mut t = native();
        t.timestamp = timestamp;
        t.expiration = expiration;
        assert_eq!(
            check(&listed(), &t).unwrap_err().code,
            ErrorCode::ValidationError
        );
    }
    let mut t = native();
    t.expiration = NOW + 300_000;
    check(&listed(), &t).unwrap();
    t.timestamp = NOW - 120_000;
    t.expiration = NOW - 60_000;
    check(&listed(), &t).unwrap();
}

#[test]
fn unsupported_token_calls_never_offer_a_policy_edit() {
    let mut cases = Vec::new();
    for selector in [
        [0x09, 0x5e, 0xa7, 0xb3],
        [0x23, 0xb8, 0x72, 0xdd],
        [0xff; 4],
    ] {
        let mut t = token();
        if let Contract::TriggerSmart { data, .. } = &mut t.contract {
            data[..4].copy_from_slice(&selector);
        }
        cases.push(t);
    }
    for mode in 0..4 {
        let mut t = token();
        if let Contract::TriggerSmart {
            data,
            contract_address,
        } = &mut t.contract
        {
            match mode {
                0 => data.push(0),
                1 => data[4] = 1,
                2 => *contract_address = owner(),
                _ => data[16..36]
                    .copy_from_slice(&address::parse(registry::TOKENS[0].address).unwrap()[1..]),
            }
        }
        cases.push(t);
    }
    for t in cases {
        let e = check(&listed(), &t).unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistDenied);
        assert!(!e.details.contains_key("command"));
    }
}

#[test]
fn fees_follow_action_and_missing_caps_name_a_positive_trx_amount() {
    let mut l = listed();
    let mut t = token();
    t.fee_limit = Some(DEFAULT_CAP + 1);
    let e = check(&l, &t).unwrap_err();
    assert_eq!(e.code, ErrorCode::FeeCapExceeded);
    assert_eq!(
        e.details["command"],
        "zwallet whitelist fee-cap tron 30.000001"
    );
    assert_eq!(e.details["feeWei"], "30000001");
    assert_eq!(e.details["capWei"], "30000000");
    l.entries.clear();
    let e = check(&l, &t).unwrap_err();
    assert_eq!(e.code, ErrorCode::WhitelistDenied);
    assert!(e.details["command"].as_str().unwrap().contains(" add "));
    for bad in [None, Some("030000000"), Some("bad")] {
        let mut l = listed();
        if let Some(v) = bad {
            l.policy.max_fee_wei.insert("tron".into(), v.into());
        } else {
            l.policy.max_fee_wei.remove("tron");
        }
        for t in [native(), token()] {
            let e = check(&l, &t).unwrap_err();
            assert_eq!(e.code, ErrorCode::FeeCapExceeded);
            assert_eq!(e.details["command"], "zwallet whitelist fee-cap tron 30");
            assert!(!e.details.contains_key("capWei"));
        }
    }
}

#[test]
fn signed_transaction_limits_apply_to_sun_token_units_and_exact_calldata_bytes() {
    let mut l = listed();
    l.policy.transaction_limits.insert(
        "tron".into(),
        TransactionLimits {
            max_native_wei: Some("999999".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        check(&l, &native()).unwrap_err().code,
        ErrorCode::WhitelistDenied
    );
    l.policy
        .transaction_limits
        .get_mut("tron")
        .unwrap()
        .max_native_wei = Some("1000000".into());
    check(&l, &native()).unwrap();
    check(&l, &token()).unwrap();
    l.policy
        .transaction_limits
        .get_mut("tron")
        .unwrap()
        .max_calldata_bytes = Some(67);
    assert_eq!(
        check(&l, &token()).unwrap_err().code,
        ErrorCode::WhitelistDenied
    );
    check(&l, &native()).unwrap();
    let limits = l.policy.transaction_limits.get_mut("tron").unwrap();
    limits.max_calldata_bytes = Some(68);
    limits.max_token_base_units.insert(
        body(&address::parse(registry::TOKENS[0].address).unwrap()),
        "999999".into(),
    );
    assert_eq!(
        check(&l, &token()).unwrap_err().code,
        ErrorCode::WhitelistDenied
    );
    *l.policy
        .transaction_limits
        .get_mut("tron")
        .unwrap()
        .max_token_base_units
        .values_mut()
        .next()
        .unwrap() = "1000000".into();
    check(&l, &token()).unwrap();
}

#[test]
fn family_signer_returns_only_a_signature_over_its_own_decoded_bytes() {
    let seed = Seed::from_phrase(zeroize::Zeroizing::new(PHRASE.to_string())).unwrap();
    let chain = crate::chains::find("tron").unwrap();
    let mut t = native();
    let signer = signer_at(&seed, 0).unwrap();
    t.owner[1..].copy_from_slice(signer.address().as_slice());
    t.timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    t.expiration = t.timestamp + 60_000;
    let raw = super::super::tx::encode_raw(&t).unwrap();
    let value = json!(format!("0x{}", hex::encode(&raw)));
    let result = Tron
        .sign_tx(&seed, 0, chain, &value, &ListView::Usable(&listed()))
        .unwrap();
    assert_eq!(result["rawData"], value);
    assert_eq!(result["txHash"], hex::encode(super::super::tx::txid(&raw)));
    let sig = hex::decode(result["signature"].as_str().unwrap()).unwrap();
    assert_eq!(super::super::tx::recover(&raw, &sig).unwrap(), t.owner);
    for view in [ListView::NoList, ListView::Unusable("tampered".into())] {
        assert!(Tron.sign_tx(&seed, 0, chain, &value, &view).is_err());
    }
    assert!(
        Tron.sign_tx(
            &seed,
            0,
            chain,
            &json!({"txHash":result["txHash"]}),
            &ListView::Usable(&listed())
        )
        .is_err()
    );
}

#[test]
fn an_evm_twin_of_a_tron_payee_cannot_receive_native_or_registry_tokens() {
    let chain = crate::chains::find("base").unwrap();
    let to = body(&pay_to());
    let seed = Seed::from_phrase(zeroize::Zeroizing::new(PHRASE.to_string())).unwrap();
    let native = UnsignedEvmTx {
        chain_id: 8453,
        nonce: 0,
        gas_limit: 21000,
        fees: EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(1),
            max_priority_fee_per_gas: U128::from(1),
        },
        to: to.parse().unwrap(),
        value: U256::from(1),
        input: Default::default(),
    };
    let mut erc20 = native.clone();
    erc20.to = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
        .parse()
        .unwrap();
    erc20.value = U256::ZERO;
    let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
    data.extend([0; 12]);
    data.extend(&pay_to()[1..]);
    data.extend(U256::from(1).to_be_bytes::<32>());
    erc20.input = data.into();
    for tx in [native, erc20] {
        let value = json!(format!(
            "0x{}",
            hex::encode(crate::chains::evm::tx::encoded_for_signing(&tx))
        ));
        let e = Evm
            .sign_tx(&seed, 0, chain, &value, &ListView::Usable(&listed()))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::WhitelistDenied);
        assert!(!e.details.contains_key("command"));
    }
}

/// Security-boundary corpus, separate from the codec's private tests. Every nested edit
/// repairs enclosing lengths so the daemon must reject the changed field itself.
pub(crate) fn rejected_codec_corpus() -> Vec<Vec<u8>> {
    fn read(input: &mut &[u8]) -> u64 {
        let mut n = 0;
        for shift in (0..70).step_by(7) {
            let b = input[0];
            *input = &input[1..];
            n |= u64::from(b & 127) << shift;
            if b & 128 == 0 {
                return n;
            }
        }
        panic!("valid fixture varint")
    }
    fn put(out: &mut Vec<u8>, mut n: u64) {
        while n >= 128 {
            out.push((n as u8 & 127) | 128);
            n >>= 7;
        }
        out.push(n as u8);
    }
    fn first_len(input: &[u8]) -> usize {
        let mut rest = input;
        let key = read(&mut rest);
        if key & 7 == 0 {
            read(&mut rest);
        } else {
            let n = read(&mut rest) as usize;
            rest = &rest[n..];
        }
        input.len() - rest.len()
    }
    fn nested(input: &[u8], path: &[u64], edit: &dyn Fn(&mut Vec<u8>)) -> Vec<u8> {
        if path.is_empty() {
            let mut out = input.to_vec();
            edit(&mut out);
            return out;
        }
        let mut rest = input;
        let mut out = Vec::new();
        while !rest.is_empty() {
            let key = read(&mut rest);
            put(&mut out, key);
            if key & 7 == 0 {
                put(&mut out, read(&mut rest));
            } else {
                let len = read(&mut rest) as usize;
                let (value, tail) = rest.split_at(len);
                rest = tail;
                let value = if key >> 3 == path[0] {
                    nested(value, &path[1..], edit)
                } else {
                    value.to_vec()
                };
                put(&mut out, value.len() as u64);
                out.extend(value);
            }
        }
        out
    }
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/tron-unsigned.json"
    ))
    .unwrap();
    let mut corpus = Vec::new();
    for (i, vector) in vectors["vectors"].as_array().unwrap().iter().enumerate() {
        let raw = hex::decode(vector["raw_data_hex"].as_str().unwrap()).unwrap();
        for (path, fields) in [
            (vec![], vec![3, 9, 10, 12]),
            (vec![11], vec![3, 4, 5]),
            (vec![11, 2], vec![3]),
            (
                vec![11, 2, 2],
                if i == 0 { vec![4] } else { vec![3, 5, 6, 7] },
            ),
        ] {
            for field in fields {
                for wire in [0, 2] {
                    corpus.push(nested(&raw, &path, &|v| {
                        put(v, field * 8 + wire);
                        if wire == 0 {
                            put(v, 2);
                        } else {
                            put(v, 1);
                            v.push(b'x');
                        }
                    }));
                }
            }
        }
        for path in [vec![], vec![11], vec![11, 2], vec![11, 2, 2]] {
            for mutation in 0..6 {
                corpus.push(nested(&raw, &path, &|v| {
                    let len = first_len(v);
                    let first = v[..len].to_vec();
                    match mutation {
                        0 => {
                            v.splice(0..0, first);
                        }
                        1 => {
                            v.drain(..len);
                            v.extend(first);
                        }
                        2 => v[0] ^= 2,
                        3 => {
                            v[0] |= 128;
                            v.insert(1, 0);
                        }
                        4 => {
                            v[1] |= 128;
                            v.insert(2, 0);
                        }
                        _ => v.push(0),
                    }
                }));
            }
        }
        for kind in [2, 30, 46, 54] {
            corpus.push(nested(&raw, &[11], &|v| v[1] = kind));
        }
        corpus.push(nested(&raw, &[11, 2, 1], &|v| v[0] ^= 1));
        for field in [1, 2] {
            corpus.push(nested(&raw, &[11, 2, 2, field], &|v| v[0] = 0xa0));
        }
        for field in [1, 4] {
            corpus.push(nested(&raw, &[field], &|v| {
                v.pop();
            }));
        }
        // Memo in its field order, and a second complete contract with matching lengths.
        let mut offset = 0;
        for _ in 0..3 {
            offset += first_len(&raw[offset..]);
        }
        let mut memo = raw.clone();
        memo.splice(offset..offset, [0x52, 1, b'x']);
        corpus.push(memo);
        let contract_len = first_len(&raw[offset..]);
        let mut twice = raw.clone();
        twice.splice(
            offset..offset,
            raw[offset..offset + contract_len].iter().copied(),
        );
        corpus.push(twice);
        corpus.push(raw[..raw.len() - 1].to_vec());
        if i == 0 {
            let mut fee = raw.clone();
            put(&mut fee, 18 * 8);
            put(&mut fee, 30_000_000);
            corpus.push(fee);
        }
    }
    corpus.push(vec![0; 1025]);
    corpus
}

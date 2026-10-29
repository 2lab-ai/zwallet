use super::*;
use crate::core::{
    error::ErrorCode,
    seed::{Seed, TEST_MNEMONIC},
};
use zeroize::Zeroizing;

fn unsigned() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("../../../tests/fixtures/tron-unsigned.json"))
        .unwrap()["vectors"]
        .as_array()
        .unwrap()
        .clone()
}

fn fixture_raw(index: usize) -> Vec<u8> {
    alloy::hex::decode(unsigned()[index]["raw_data_hex"].as_str().unwrap()).unwrap()
}

fn rejects(raw: &[u8]) {
    let error = decode_raw(raw).unwrap_err();
    assert_eq!(error.code, ErrorCode::ValidationError);
    assert!(
        error.message.contains("refusing to sign"),
        "{}",
        error.message
    );
}

#[test]
fn tr12_node_golden_vectors_decode() {
    for (i, v) in unsigned().iter().enumerate() {
        let raw = fixture_raw(i);
        let tx = decode_raw(&raw).unwrap();
        assert_eq!(encode_raw(&tx).unwrap(), raw);
        assert_eq!(alloy::hex::encode(txid(&raw)), v["txID"]);
        assert_eq!(address::encode(&tx.owner), v["owner"]);
        assert_eq!(tx.ref_block_bytes, [0x10, 0x08]);
        assert_eq!(alloy::hex::encode(tx.ref_block_hash), "a7a95ebd17ea3b6f");
        assert_eq!(tx.expiration, 1793157384000);
        let summary = tx.to_json().unwrap();
        assert_eq!(summary["txHash"], v["txID"]);
        assert_eq!(summary["to"], v["to"]);
        assert_eq!(summary["amount"], v["amount"]);
        if i == 0 {
            assert_eq!(tx.timestamp, 1793157325526);
            assert_eq!(tx.fee_limit, None);
            assert!(matches!(
                tx.contract,
                Contract::Transfer { amount_sun: 1, .. }
            ));
            assert_eq!(summary["kind"], "native-transfer");
        } else {
            assert_eq!(tx.timestamp, 1793157325924);
            assert_eq!(tx.fee_limit, Some(30_000_000));
            assert_eq!(summary["token"], "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t");
            assert_eq!(summary["assetType"], "unverified");
        }
    }
}

#[test]
fn tr12_new_signatures_recover_to_test_key_and_are_low_s_with_v_0_or_1() {
    let seed = Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap();
    let signer = super::super::signer_at(&seed, 0).unwrap();
    for i in 0..2 {
        let mut tx = decode_raw(&fixture_raw(i)).unwrap();
        assert!(sign(&signer, &tx).is_err());
        tx.owner = address::parse("TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6").unwrap();
        let raw = encode_raw(&tx).unwrap();
        let signature = sign(&signer, &tx).unwrap();
        assert!(signature[64] <= 1);
        assert!(
            Signature::from_raw(&signature)
                .unwrap()
                .normalize_s()
                .is_none()
        );
        assert_eq!(recover(&raw, &signature).unwrap(), tx.owner);
        let envelope = encode_signed(&raw, &signature).unwrap();
        let mut r = Reader::new(&envelope);
        assert_eq!(r.bytes(1).unwrap(), raw);
        assert_eq!(r.bytes(2).unwrap(), signature);
        r.finish().unwrap();
        let mut changed = raw.clone();
        changed[3] ^= 1;
        assert_ne!(recover(&changed, &signature).unwrap(), tx.owner);
        assert!(encode_signed(&changed, &signature).is_err());
    }
}

#[test]
fn tr12_mainnet_signatures_recover_both_legacy_v_and_high_s() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/tron-signed.json")).unwrap();
    for (i, v) in fixture["vectors"].as_array().unwrap().iter().enumerate() {
        let raw = alloy::hex::decode(v["raw_data_hex"].as_str().unwrap()).unwrap();
        let signature = alloy::hex::decode(v["signature"].as_str().unwrap()).unwrap();
        assert_eq!(alloy::hex::encode(txid(&raw)), v["txID"]);
        let owner = recover(&raw, &signature).unwrap();
        assert_eq!(address::encode(&owner), v["owner"]);
        assert_eq!(signature[64], if i == 0 { 27 } else { 0 });
        assert_eq!(
            Signature::from_raw(&signature)
                .unwrap()
                .normalize_s()
                .is_some(),
            i == 1
        );
        assert_eq!(decode_raw(&raw).unwrap().owner, owner);
        assert_eq!(
            encode_signed(&raw, &signature).unwrap().len() + 64,
            if i == 0 { 345 } else { 267 }
        );
    }
}

/// Edit one nested protobuf message, updating every enclosing length. Refusal must be for the
/// mutation itself rather than an incidental stale enclosing length.
fn nested(raw: &[u8], path: &[u64], edit: &dyn Fn(&mut Vec<u8>)) -> Vec<u8> {
    if path.is_empty() {
        let mut v = raw.to_vec();
        edit(&mut v);
        return v;
    }
    let mut r = Reader::new(raw);
    let mut out = Vec::new();
    while !r.rest.is_empty() {
        let tag = r.varint().unwrap();
        let field = tag >> 3;
        if tag & 7 == 0 {
            uint(&mut out, field, r.varint().unwrap());
        } else {
            assert_eq!(tag & 7, 2);
            let len = r.varint().unwrap() as usize;
            let value = &r.rest[..len];
            r.rest = &r.rest[len..];
            let value = if field == path[0] {
                nested(value, &path[1..], edit)
            } else {
                value.to_vec()
            };
            bytes(&mut out, field, &value);
        }
    }
    out
}

#[test]
fn tr13_unknown_fields_at_every_level_are_refused() {
    for i in 0..2 {
        let raw = fixture_raw(i);
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
                    let altered = nested(&raw, &path, &|v| {
                        if wire == 0 {
                            uint(v, field, 2);
                        } else {
                            bytes(v, field, b"memo");
                        }
                    });
                    rejects(&altered);
                }
            }
        }
        // Memo at its proper position before contract field 11, with correct lengths.
        let mut memo = raw.clone();
        let mut prefix = Reader::new(&raw);
        prefix.bytes(1).unwrap();
        prefix.bytes(4).unwrap();
        prefix.uint(8).unwrap();
        let offset = raw.len() - prefix.rest.len();
        memo.splice(offset..offset, [0x52, 0x01, b'x']);
        rejects(&memo);
    }
}

#[test]
fn tr13_duplicate_reordered_wrong_wire_nonminimal_and_trailing_at_every_level() {
    for i in 0..2 {
        let raw = fixture_raw(i);
        for path in [vec![], vec![11], vec![11, 2], vec![11, 2, 2]] {
            for mutation in 0..6 {
                let altered = nested(&raw, &path, &|v| {
                    let mut reader = Reader::new(v);
                    let tag = reader.varint().unwrap();
                    if tag & 7 == 0 {
                        reader.varint().unwrap();
                    } else {
                        let len = reader.varint().unwrap() as usize;
                        reader.rest = &reader.rest[len..];
                    }
                    let first_len = v.len() - reader.rest.len();
                    let first = v[..first_len].to_vec();
                    match mutation {
                        0 => {
                            v.splice(0..0, first);
                        }
                        1 => {
                            v.drain(..first_len);
                            v.extend(first);
                        }
                        2 => v[0] ^= 2,
                        3 => {
                            v[0] |= 0x80;
                            v.insert(1, 0);
                        }
                        4 => {
                            v[1] |= 0x80;
                            v.insert(2, 0);
                        }
                        _ => v.push(0),
                    }
                });
                rejects(&altered);
            }
        }
    }
}

#[test]
fn tr13_contract_type_and_any_type_url_are_not_interchangeable() {
    let raw = fixture_raw(0);
    for kind in [0, 2, 30, 46, 54, 127] {
        let altered = nested(&raw, &[11], &|v| {
            v[1] = kind;
        });
        rejects(&altered);
        if kind == 46 {
            assert!(
                decode_raw(&altered)
                    .unwrap_err()
                    .message
                    .contains("hands this account to other keys")
            );
        }
    }
    for i in 0..2 {
        rejects(&nested(&fixture_raw(i), &[11, 2, 1], &|v| {
            v[0] ^= 1;
        }));
    }
}

#[test]
fn tr13_all_truncations_oversized_lengths_and_overflow_varints_are_refused() {
    for i in 0..2 {
        let raw = fixture_raw(i);
        for end in 0..raw.len() {
            // Removing a complete optional fee field gives protobuf's canonical zero default.
            let candidate = &raw[..end];
            if let Ok(tx) = decode_raw(candidate) {
                assert_eq!(i, 1);
                assert_eq!(tx.fee_limit, None);
                assert_eq!(encode_raw(&tx).unwrap(), candidate);
            } else {
                rejects(candidate);
            }
        }
        for path in [vec![], vec![11], vec![11, 2], vec![11, 2, 2]] {
            rejects(&nested(&raw, &path, &|v| {
                v.splice(1..2, [0xff; 10]);
            }));
        }
    }
    rejects(&vec![0; MAX_RAW_BYTES + 1]);
    for last in [2, 127, 128, 255] {
        let mut input = vec![0xff; 9];
        input.push(last);
        assert!(Reader::new(&input).varint().is_err());
    }
}

#[test]
fn tr13_amount_is_positive_int64_and_its_ten_byte_alias_is_refused() {
    let raw = fixture_raw(0);
    // Canonical amount1 [0x18,0x01] to a 10-byte alias, with all nested lengths fixed.
    rejects(&nested(&raw, &[11, 2, 2], &|v| {
        v.pop();
        v.extend([0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0]);
    }));
    for amount in [0, 1u64 << 63, u64::MAX] {
        rejects(&nested(&raw, &[11, 2, 2], &|v| {
            v.truncate(v.len() - 2);
            uint(v, 3, amount);
        }));
    }
    let mut tx = decode_raw(&raw).unwrap();
    let Contract::Transfer { amount_sun, .. } = &mut tx.contract else {
        unreachable!()
    };
    *amount_sun = i64::MAX as u64;
    assert_eq!(decode_raw(&encode_raw(&tx).unwrap()).unwrap(), tx);
    tx.fee_limit = Some(1);
    assert!(encode_raw(&tx).is_err());
    let mut fee = raw.clone();
    uint(&mut fee, 18, 1);
    rejects(&fee);
}

#[test]
fn tr13_address_lengths_and_prefixes_are_checked_for_owner_and_target() {
    for i in 0..2 {
        for field in [1, 2] {
            let raw = fixture_raw(i);
            let path = [11, 2, 2, field];
            rejects(&nested(&raw, &path, &|v| {
                v[0] = 0xa0;
            }));
            rejects(&nested(&raw, &path, &|v| {
                v.pop();
            }));
            rejects(&nested(&raw, &path, &|v| {
                v.push(0);
            }));
        }
    }
    for field in [1, 4] {
        rejects(&nested(&fixture_raw(0), &[field], &|v| {
            v.push(0);
        }));
    }
}

#[test]
fn a_transfer_meaning_requires_exact_canonical_abi() {
    let canonical = decode_raw(&fixture_raw(1)).unwrap();
    for mutation in 0..5 {
        let mut tx = canonical.clone();
        let Contract::TriggerSmart { data, .. } = &mut tx.contract else {
            unreachable!()
        };
        match mutation {
            0 => data[4] = 1,
            1 => data.extend([0; 32]),
            2 => {
                data.pop();
            }
            3 => data[..4].copy_from_slice(&[0x09, 0x5e, 0xa7, 0xb3]),
            _ => data[..4].copy_from_slice(&[0x23, 0xb8, 0x72, 0xdd]),
        }
        let decoded = decode_raw(&encode_raw(&tx).unwrap()).unwrap();
        assert!(decoded.transfer_call().is_none());
        assert_eq!(decoded.to_json().unwrap()["kind"], "contract-call");
    }
}

#[test]
fn signature_recovery_refuses_bad_lengths_scalars_and_non_tron_v() {
    let raw = fixture_raw(0);
    for len in [0, 64, 66] {
        assert!(recover(&raw, &vec![0; len]).is_err());
    }
    for v in [0, 1, 2, 26, 27, 28, 29, 35, 37, 255] {
        let mut sig = [0; 65];
        sig[64] = v;
        assert!(recover(&raw, &sig).is_err());
    }
    let fixture: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/tron-signed.json")).unwrap();
    let mut signature =
        alloy::hex::decode(fixture["vectors"][0]["signature"].as_str().unwrap()).unwrap();
    for v in [2, 26, 29, 35, 37, 255] {
        signature[64] = v;
        assert!(recover(&raw, &signature).is_err());
    }
}

#[test]
fn tr13_a_second_contract_and_explicit_zero_fee_are_refused() {
    for i in 0..2 {
        let raw = fixture_raw(i);
        let mut r = Reader::new(&raw);
        r.bytes(1).unwrap();
        r.bytes(4).unwrap();
        r.uint(8).unwrap();
        let start = raw.len() - r.rest.len();
        r.bytes(11).unwrap();
        let end = raw.len() - r.rest.len();
        let mut two = raw.clone();
        two.splice(end..end, raw[start..end].iter().copied());
        rejects(&two);

        r.uint(14).unwrap();
        let without_fee = raw.len() - r.rest.len();
        let mut zero_fee = raw[..without_fee].to_vec();
        uint(&mut zero_fee, 18, 0);
        rejects(&zero_fee);
    }
}

#[test]
fn tr13_one_kib_limit_is_inclusive_and_checked_before_nested_allocation() {
    let raw = fixture_raw(1);
    let mut boundary = None;
    for length in 68..MAX_RAW_BYTES {
        let input = nested(&raw, &[11, 2, 2, 4], &|v| v.resize(length, 0));
        if input.len() == MAX_RAW_BYTES {
            let decoded = decode_raw(&input).unwrap();
            assert_eq!(encode_raw(&decoded).unwrap(), input);
            boundary = Some(length);
            break;
        }
    }
    let length = boundary.expect("a valid raw transaction can be exactly 1 KiB");
    let too_large = nested(&raw, &[11, 2, 2, 4], &|v| v.resize(length + 1, 0));
    assert_eq!(too_large.len(), MAX_RAW_BYTES + 1);
    rejects(&too_large);
}

#[test]
fn malformed_byte_corpus_never_panics_and_accepted_mutations_remain_canonical() {
    // Deterministic input corpus: no new fuzzing dependency or unbounded test duration.
    let mut state = 0x8818_726a_510e_83f9u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
    };
    for length in 0..=MAX_RAW_BYTES + 1 {
        let input: Vec<u8> = (0..length).map(|_| next()).collect();
        if let Ok(tx) = decode_raw(&input) {
            assert_eq!(encode_raw(&tx).unwrap(), input);
        }
    }
    for i in 0..2 {
        let raw = fixture_raw(i);
        for position in 0..raw.len() {
            for replacement in [0, 0x80, 0xff, next()] {
                let mut input = raw.clone();
                input[position] = replacement;
                if let Ok(tx) = decode_raw(&input) {
                    assert_eq!(encode_raw(&tx).unwrap(), input);
                }
            }
        }
    }
}

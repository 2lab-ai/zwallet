//! Adversarial requests through a real isolated key-daemon socket. No RPC or broadcast.
//! Successful controls are inspected in memory; no raw signed bytes are printed.
mod common;

use alloy::{hex, primitives::U256};
use common::{Home, PW};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zwallet::{
    chains::{
        self, ChainFamily,
        solana::{self, Solana},
        ton::{self, Ton},
    },
    core::{
        error::ErrorCode,
        protocol::{Request, Response},
        whitelist::{Entry, TransactionLimits, Whitelist},
    },
};

const AMOUNT: u64 = 1_234_567_890;
fn cached(home: &Home, chain: &str) -> String {
    let file: Value = serde_json::from_slice(&std::fs::read(home.wallet_json()).unwrap()).unwrap();
    file["keys"][0]["addresses"][chain].as_str().unwrap().into()
}
fn request(home: &Home, chain: &str, unsigned: Value) -> Response {
    let mut socket = UnixStream::connect(home.sock()).expect("connect to isolated daemon");
    socket
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut bytes = serde_json::to_vec(&Request::SignTx {
        chain: chain.into(),
        index: 0,
        tx: unsigned,
    })
    .unwrap();
    bytes.push(b'\n');
    socket.write_all(&bytes).unwrap();
    socket.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = String::new();
    BufReader::new(socket)
        .read_line(&mut response)
        .expect("bounded daemon response");
    serde_json::from_str(&response).expect("daemon response shape")
}
fn refused(home: &Home, chain: &str, unsigned: Value, code: ErrorCode) {
    let response = request(home, chain, unsigned);
    assert!(
        !response.ok,
        "a prohibited request unexpectedly returned success"
    );
    assert_eq!(response.error_code, Some(code));
    assert!(
        response.result.is_none(),
        "a refusal must not return signed data"
    );
    for key in ["rawTx", "signature", "signedTx", "txHash"] {
        assert!(
            !response.details.contains_key(key),
            "a refusal included signed-result metadata"
        );
    }
}
fn edit(home: &Home, change: impl FnOnce(&mut Whitelist)) {
    home.stop_daemons();
    home.edit_whitelist(change);
    home.unlock("10m");
}
fn scoped_payee(home: &Home, chain: &str, to: &str, fee: u64) {
    let chain = chains::find(chain).unwrap();
    let target = chains::ed25519::policy_address(chain, to).unwrap();
    edit(home, |list| {
        list.entries.clear();
        list.add(Entry::new_address("bob", &target, &[chain.key]))
            .unwrap();
        list.policy.set_max_fee_wei(chain.key, U256::from(fee));
    });
}
fn wrong_scope(home: &Home) {
    edit(home, |list| {
        list.add(Entry::new_address(
            "evm-only",
            "0x1111111111111111111111111111111111111111",
            &["*"],
        ))
        .unwrap();
    });
}
fn sol_payload(from: &str, to: &str, amount: u64) -> Value {
    let transfer = solana::tx::Transfer {
        from: solana::parse_address(from).unwrap(),
        to: solana::parse_address(to).unwrap(),
        blockhash: [9; 32],
        lamports: amount,
    };
    json!(format!(
        "0x{}",
        hex::encode(solana::tx::encode(&transfer).unwrap())
    ))
}
fn changed_hex(unsigned: &Value, change: impl FnOnce(&mut Vec<u8>)) -> Value {
    let mut bytes = hex::decode(unsigned.as_str().unwrap().strip_prefix("0x").unwrap()).unwrap();
    change(&mut bytes);
    json!(format!("0x{}", hex::encode(bytes)))
}
fn tampered_list_refuses(home: &Home, chain: &str, unsigned: Value) {
    let path = home.path().join("whitelist.json");
    let original = std::fs::read(&path).unwrap();
    let mut file: Value = serde_json::from_slice(&original).unwrap();
    file["entries"][0]["nickname"] = json!("mallory");
    common::replace_file(&path, &serde_json::to_vec(&file).unwrap());
    refused(home, chain, unsigned, ErrorCode::WhitelistInvalid);
    home.stop_daemons();
    common::replace_file(&path, &original);
}
#[test]
fn real_solana_daemon_rejects_untrusted_policy_and_changed_instruction_bytes() {
    let home = Home::new();
    home.create("main", &[]);
    let from = cached(&home, "solana");
    let to = bs58::encode([2; 32]).into_string();
    let unsigned = || sol_payload(&from, &to, AMOUNT);
    home.unlock("10m");
    refused(&home, "solana", unsigned(), ErrorCode::WhitelistInvalid);
    wrong_scope(&home);
    refused(&home, "solana", unsigned(), ErrorCode::WhitelistDenied);
    scoped_payee(&home, "solana", &to, 5000);
    let allowed = request(&home, "solana", unsigned());
    assert!(allowed.ok, "a valid exact-chain transfer must sign");
    let signed = Solana
        .inspect_signed(
            chains::find("solana").unwrap(),
            allowed.result.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(signed.signer, from);
    solana::verify_signed_payload(&signed, &unsigned()).unwrap();
    // Header account privileges, instruction opcode, extra instructions/trailing bytes and
    // sender substitution are all rejected even though the recipient is allowlisted.
    for malformed in [
        changed_hex(&unsigned(), |b| {
            b[0] = 2;
        }),
        changed_hex(&unsigned(), |b| {
            b[138] = 9;
        }),
        changed_hex(&unsigned(), |b| {
            b.push(0);
        }),
        changed_hex(&unsigned(), |b| {
            b.truncate(149);
        }),
        sol_payload(&bs58::encode([3; 32]).into_string(), &to, AMOUNT),
    ] {
        refused(&home, "solana", malformed, ErrorCode::ValidationError);
    }
    refused(
        &home,
        "solana",
        sol_payload(&from, &bs58::encode([4; 32]).into_string(), AMOUNT),
        ErrorCode::WhitelistDenied,
    );
    edit(&home, |list| {
        list.policy.transaction_limits.insert(
            "solana".into(),
            TransactionLimits {
                max_native_wei: Some((AMOUNT - 1).to_string()),
                ..Default::default()
            },
        );
    });
    refused(&home, "solana", unsigned(), ErrorCode::WhitelistDenied);
    edit(&home, |list| {
        list.policy
            .transaction_limits
            .get_mut("solana")
            .unwrap()
            .max_native_wei = Some(AMOUNT.to_string());
        list.policy.set_max_fee_wei("solana", U256::from(4999));
    });
    refused(&home, "solana", unsigned(), ErrorCode::FeeCapExceeded);
    edit(&home, |list| {
        list.policy.max_fee_wei.remove("solana");
    });
    refused(&home, "solana", unsigned(), ErrorCode::FeeCapExceeded);
    edit(&home, |list| {
        list.policy.set_max_fee_wei("solana", U256::from(5000));
    });
    assert!(
        request(&home, "solana", unsigned()).ok,
        "exact cap boundary must remain allowed"
    );
    tampered_list_refuses(&home, "solana", unsigned());
}

fn put_bits(bytes: &mut [u8], offset: usize, value: &[u8]) {
    for bit in 0..value.len() * 8 {
        let position = offset + bit;
        let mask = 1 << (7 - position % 8);
        if value[bit / 8] & (1 << (7 - bit % 8)) != 0 {
            bytes[position / 8] |= mask;
        } else {
            bytes[position / 8] &= !mask;
        }
    }
}
/// The @ton/ton fixture uses two cells, no CRC/index, a 112-bit signing cell, and a 416-bit
/// internal message with a 4-byte coin amount. Patch only public semantic fields, independently
/// of zwallet's BOC encoder. Fixed layout guards make fixture changes explicit.
fn ton_payload(to: [u8; 32], amount: u32) -> Value {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/ton/v4r2-sdk.json")).unwrap();
    let mut bytes = hex::decode(fixture["unsignedBoc"].as_str().unwrap()).unwrap();
    assert_eq!(
        &bytes[..13],
        &[0xb5, 0xee, 0x9c, 0x72, 1, 1, 2, 1, 0, 0x47, 0, 1, 0x1c]
    );
    assert_eq!(&bytes[13..17], &ton::TON_WALLET_ID.to_be_bytes());
    assert_eq!(&bytes[25..30], &[0, 3, 1, 0, 0x68]);
    let deadline = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 240;
    bytes[17..21].copy_from_slice(&(deadline as u32).to_be_bytes());
    // Child data starts at byte30: 6 header bits + 2 addr tag + 1 anycast + 8 workchain.
    put_bits(&mut bytes, 30 * 8 + 17, &to);
    // Address ends after 273 child bits, followed by a 4-bit length of four amount bytes.
    assert!((1 << 24..u32::MAX).contains(&amount));
    put_bits(&mut bytes, 30 * 8 + 277, &amount.to_be_bytes());
    let value = json!(format!("0x{}", hex::encode(bytes)));
    let decoded = ton::inspect_unsigned(chains::find("ton").unwrap(), &value).unwrap();
    assert_eq!(decoded["to"], format!("0:{}", hex::encode(to)));
    assert_eq!(decoded["amountRaw"], amount.to_string());
    value
}
#[test]
fn real_ton_daemon_rejects_untrusted_policy_unsupported_modes_and_expired_bytes() {
    let home = Home::new();
    home.create("main", &[]);
    let from = cached(&home, "ton");
    let to = format!("0:{}", "11".repeat(32));
    let unsigned = || ton_payload([0x11; 32], AMOUNT as u32);
    home.unlock("10m");
    refused(&home, "ton", unsigned(), ErrorCode::WhitelistInvalid);
    wrong_scope(&home);
    refused(&home, "ton", unsigned(), ErrorCode::WhitelistDenied);
    scoped_payee(&home, "ton", &to, ton::FEE_RESERVE);
    let exact = unsigned();
    let allowed = request(&home, "ton", exact.clone());
    assert!(allowed.ok, "a valid exact-chain transfer must sign");
    let signed = Ton
        .inspect_signed(
            chains::find("ton").unwrap(),
            allowed.result.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(signed.signer, from);
    ton::verify_signed_payload(&signed, &exact).unwrap();
    for malformed in [
        changed_hex(&unsigned(), |b| {
            b[25] = 1;
        }), // V4 plugin operation.
        changed_hex(&unsigned(), |b| {
            b[26] = 128;
        }), // Sweep-balance mode.
        changed_hex(&unsigned(), |b| {
            b[13] ^= 1;
        }), // Another wallet id.
        changed_hex(&unsigned(), |b| {
            b[17..21].fill(0);
        }), // Expired.
        changed_hex(&unsigned(), |b| {
            b.push(0);
        }),
        changed_hex(&unsigned(), |b| {
            b.truncate(b.len() - 1);
        }),
    ] {
        refused(&home, "ton", malformed, ErrorCode::ValidationError);
    }
    refused(
        &home,
        "ton",
        ton_payload([0x22; 32], AMOUNT as u32),
        ErrorCode::WhitelistDenied,
    );
    edit(&home, |list| {
        list.policy.transaction_limits.insert(
            "ton".into(),
            TransactionLimits {
                max_native_wei: Some((AMOUNT - 1).to_string()),
                ..Default::default()
            },
        );
    });
    refused(&home, "ton", unsigned(), ErrorCode::WhitelistDenied);
    edit(&home, |list| {
        list.policy
            .transaction_limits
            .get_mut("ton")
            .unwrap()
            .max_native_wei = Some(AMOUNT.to_string());
        list.policy
            .set_max_fee_wei("ton", U256::from(ton::FEE_RESERVE - 1));
    });
    refused(&home, "ton", unsigned(), ErrorCode::FeeCapExceeded);
    edit(&home, |list| {
        list.policy.max_fee_wei.remove("ton");
    });
    refused(&home, "ton", unsigned(), ErrorCode::FeeCapExceeded);
    edit(&home, |list| {
        list.policy
            .set_max_fee_wei("ton", U256::from(ton::FEE_RESERVE));
    });
    assert!(
        request(&home, "ton", unsigned()).ok,
        "exact amount/reserve boundary must remain allowed"
    );
    tampered_list_refuses(&home, "ton", unsigned());
}
#[test]
fn new_family_cache_repair_requires_password_and_plain_unlock_rejects_tampering() {
    let home = Home::new();
    home.create("main", &[]);
    let original: Value =
        serde_json::from_slice(&std::fs::read(home.wallet_json()).unwrap()).unwrap();
    home.edit_wallet(|file| {
        file["keys"][0]["addresses"]["solana"] = json!(bs58::encode([5; 32]).into_string());
        file["keys"][0]["addresses"]["ton"] = json!(format!("0:{}", "55".repeat(32)));
    });
    let tampered = std::fs::read(home.wallet_json()).unwrap();
    home.run_stdin(
        &[
            "address",
            "--chain",
            "solana",
            "--verify",
            "--password-stdin",
            "--json",
        ],
        "intentionally wrong test password\n",
    )
    .err("WRONG_PASSWORD");
    assert!(
        std::fs::read(home.wallet_json()).unwrap() == tampered,
        "wrong password changed wallet bytes"
    );
    home.run_stdin(
        &["unlock", "10m", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    )
    .err("VALIDATION_ERROR");
    assert!(
        std::fs::read(home.wallet_json()).unwrap() == tampered,
        "plain unlock repaired unauthenticated metadata"
    );
    assert!(home.daemon_pids().is_empty());
    let repaired = home
        .run_stdin(
            &[
                "address",
                "--chain",
                "ton",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(repaired["verified"], true);
    let after: Value = serde_json::from_slice(&std::fs::read(home.wallet_json()).unwrap()).unwrap();
    assert!(
        after["crypto"] == original["crypto"],
        "address repair replaced sealed seed material"
    );
    assert_eq!(after["keys"], original["keys"]);
    home.unlock("10m");
    for chain in ["solana", "ton"] {
        let response = home.run(&["address", "--chain", chain, "--json"]).ok();
        assert_eq!(response["address"], original["keys"][0]["addresses"][chain]);
    }
}

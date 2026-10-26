//! Function signatures for `call` / `invoke`: `name(argTypes)(returnTypes)`, e.g.
//! `balanceOf(address)(uint256)`. Arguments are CLI strings coerced to their ABI types.

use super::parse_address;
use crate::core::error::{Result, WalletError};
use alloy::dyn_abi::{DynSolType, DynSolValue, FunctionExt, JsonAbiExt, Specifier};
use alloy::hex;
use alloy::json_abi::Function;
use alloy::primitives::Bytes;
use serde_json::Value;

/// Parses `name(types)` or `name(types)(returns)` (also accepts `... returns (...)`).
pub fn parse_function(sig: &str) -> Result<Function> {
    Function::parse(sig.trim()).map_err(|e| {
        WalletError::validation(format!(
            "invalid function signature '{sig}': {e} (expected e.g. balanceOf(address)(uint256))"
        ))
    })
}

/// Selector + ABI-encoded arguments.
pub fn encode_call(func: &Function, args: &[String]) -> Result<Bytes> {
    if args.len() != func.inputs.len() {
        return Err(WalletError::validation(format!(
            "{} takes {} argument(s), got {}",
            func.signature(),
            func.inputs.len(),
            args.len()
        )));
    }
    let mut values = Vec::with_capacity(args.len());
    for (i, (param, arg)) in func.inputs.iter().zip(args).enumerate() {
        let ty: DynSolType = param.resolve().map_err(|e| {
            WalletError::validation(format!("unsupported parameter type {}: {e}", param.ty))
        })?;
        // A top-level string is the argument exactly as given: no quote stripping, no trim.
        let value = match ty {
            DynSolType::String => Ok(DynSolValue::String(arg.clone())),
            _ => coerce(&ty, arg),
        }
        .map_err(|e| {
            WalletError::new(
                e.code,
                format!(
                    "argument {} ('{arg}') is not a valid {}: {}",
                    i + 1,
                    param.ty,
                    e.message
                ),
            )
        })?;
        values.push(value);
    }
    func.abi_encode_input(&values)
        .map(Bytes::from)
        .map_err(|e| WalletError::validation(format!("cannot encode arguments: {e}")))
}

/// Coerces one CLI argument, or one element of a list, to `ty`. Arrays (`[a, b]`) and tuples
/// (`(a, b)`) are split here, so the value that gets encoded is built by the same parser that
/// checked every leaf:
/// - `address`: `0x` plus 40 hex digits; mixed case must be a valid EIP-55 checksum (the
///   `transfer --to` rule). alloy's coercion alone ignores checksums and the `0x` prefix.
/// - `uint`/`int`: plain decimal (a leading `-` for `int`) or `0x` hex. alloy's coercion would
///   also read units, exponents and other radixes (`1 ether`, `1e18`, `0b1`).
/// - `bytes`, `bytesN`, `function`: `0x` hex (exactly N bytes for `bytesN`, 24 for `function`).
///   alloy's coercion would also take bare hex.
/// - `string` inside a list: see [`list_string`]. (A top-level string never reaches here.)
/// - `bool`: alloy's `coerce_str` (`true` or `false`).
fn coerce(ty: &DynSolType, s: &str) -> Result<DynSolValue> {
    let leaf = |s: &str| {
        ty.coerce_str(s)
            .map_err(|e| WalletError::validation(e.to_string()))
    };
    match ty {
        DynSolType::Address => parse_address(s).map(DynSolValue::Address),
        DynSolType::Uint(_) | DynSolType::Int(_) => {
            let t = s.trim();
            let signed = matches!(ty, DynSolType::Int(_));
            let digits = |b: &str, hex: bool| {
                !b.is_empty()
                    && b.bytes().all(|c| {
                        if hex {
                            c.is_ascii_hexdigit()
                        } else {
                            c.is_ascii_digit()
                        }
                    })
            };
            let plain = match t.strip_prefix("0x") {
                Some(h) => digits(h, true),
                None => digits(t.strip_prefix('-').filter(|_| signed).unwrap_or(t), false),
            };
            if !plain {
                return Err(WalletError::validation(if signed {
                    "expected a plain decimal integer (optionally negative) or 0x hex"
                } else {
                    "expected a plain decimal integer or 0x hex"
                }));
            }
            leaf(t)
        }
        DynSolType::FixedBytes(_) | DynSolType::Function | DynSolType::Bytes => {
            let t = s.trim();
            if !t.starts_with("0x") {
                return Err(WalletError::validation("expected 0x-prefixed hex"));
            }
            leaf(t)
        }
        DynSolType::String => list_string(s).map(|v| DynSolValue::String(v.to_string())),
        DynSolType::Bool => leaf(s),
        DynSolType::Array(inner) => split_list(s, '[', ']')?
            .into_iter()
            .map(|e| coerce(inner, e))
            .collect::<Result<_>>()
            .map(DynSolValue::Array),
        DynSolType::FixedArray(inner, len) => {
            let items = split_list(s, '[', ']')?;
            if items.len() != *len {
                return Err(WalletError::validation(format!(
                    "expected {len} elements, got {}",
                    items.len()
                )));
            }
            items
                .into_iter()
                .map(|e| coerce(inner, e))
                .collect::<Result<_>>()
                .map(DynSolValue::FixedArray)
        }
        DynSolType::Tuple(types) => {
            let items = split_list(s, '(', ')')?;
            if items.len() != types.len() {
                return Err(WalletError::validation(format!(
                    "expected a tuple of {}, got {} element(s)",
                    types.len(),
                    items.len()
                )));
            }
            types
                .iter()
                .zip(items)
                .map(|(t, e)| coerce(t, e))
                .collect::<Result<_>>()
                .map(DynSolValue::Tuple)
        }
    }
}

/// A string element of a list, as [`split_list`] cut it (trimmed): the element as written, or,
/// when it starts with `"` or `'`, the text between that quote and the same quote ending the
/// element. Quoting keeps commas, brackets and surrounding spaces; there are no escapes, so the
/// text may not contain its own quote (quote with the other kind instead).
fn list_string(e: &str) -> Result<&str> {
    let Some(q) = e.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
        return Ok(e);
    };
    e[1..]
        .strip_suffix(q)
        .filter(|inner| !inner.contains(q))
        .ok_or_else(|| {
            WalletError::validation(format!(
                "a list element that starts with {q} must end with {q} and not contain it (there \
                 are no escapes; quote with the other kind)"
            ))
        })
}

/// Splits `[a, b]` / `(a, b)` into its trimmed top-level elements. A `"` or `'` that starts an
/// element opens a quoted string, inside which commas and brackets are text (see
/// [`list_string`]). Unquoted elements must not contain unbalanced brackets, and an empty element
/// (e.g. a trailing comma) is refused.
fn split_list(s: &str, open: char, close: char) -> Result<Vec<&str>> {
    let bad = |why: &str| WalletError::validation(format!("expected {open}...{close}: {why}"));
    let inner = s
        .trim()
        .strip_prefix(open)
        .and_then(|r| r.strip_suffix(close))
        .ok_or_else(|| bad("missing brackets"))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    // Last non-space character outside quotes; a quote opens a string only after a separator.
    let mut prev: Option<char> = None;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
                prev = Some(c);
            }
            continue;
        }
        match c {
            '"' | '\'' if matches!(prev, None | Some('[' | '(' | ',')) => quote = Some(c),
            '[' | '(' => depth += 1,
            ']' | ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| bad("unbalanced brackets"))?
            }
            ',' if depth == 0 => {
                items.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        if !c.is_whitespace() {
            prev = Some(c);
        }
    }
    if quote.is_some() {
        return Err(bad("unterminated quote"));
    }
    if depth != 0 {
        return Err(bad("unbalanced brackets"));
    }
    items.push(inner[start..].trim());
    if items.iter().any(|e| e.is_empty()) {
        return Err(bad("empty element"));
    }
    Ok(items)
}

/// Decodes return data into JSON (scalars as strings, arrays/tuples as arrays).
pub fn decode_output(func: &Function, data: &[u8]) -> Result<Vec<Value>> {
    let values = func.abi_decode_output(data).map_err(|e| {
        // The decoder's message can quote the returned data, which the contract chose.
        WalletError::validation(format!(
            "return data (0x{}) does not match {}: {}",
            hex::encode(data),
            func.signature_with_outputs(),
            crate::core::text::ascii_inert(&e.to_string(), crate::core::text::PROSE_MAX)
        ))
    })?;
    Ok(values.iter().map(to_json).collect())
}

pub fn to_json(value: &DynSolValue) -> Value {
    match value {
        DynSolValue::Bool(b) => Value::String(b.to_string()),
        DynSolValue::Int(i, _) => Value::String(i.to_string()),
        DynSolValue::Uint(u, _) => Value::String(u.to_string()),
        DynSolValue::FixedBytes(w, size) => {
            Value::String(format!("0x{}", hex::encode(&w[..*size])))
        }
        DynSolValue::Address(a) => Value::String(a.to_checksum(None)),
        DynSolValue::Function(f) => Value::String(format!("0x{}", hex::encode(f.as_slice()))),
        DynSolValue::Bytes(b) => Value::String(format!("0x{}", hex::encode(b))),
        DynSolValue::String(s) => Value::String(s.clone()),
        DynSolValue::Array(items) | DynSolValue::FixedArray(items) | DynSolValue::Tuple(items) => {
            Value::Array(items.iter().map(to_json).collect())
        }
    }
}

/// Raw `0x` calldata given with `--data`.
pub fn parse_calldata(input: &str) -> Result<Bytes> {
    let s = input.trim();
    let hex_part = s
        .strip_prefix("0x")
        .ok_or_else(|| WalletError::validation("--data must be 0x-prefixed hex"))?;
    hex::decode(hex_part)
        .map(Bytes::from)
        .map_err(|_| WalletError::validation("--data is not valid hex"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::U256;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn encodes_erc20_transfer_exactly() {
        let f = parse_function("transfer(address,uint256)").unwrap();
        let data = encode_call(
            &f,
            &args(&["0x70997970C51812dc3A010C7d01b50e0d17dc79C8", "1000000"]),
        )
        .unwrap();
        let expected = concat!(
            "a9059cbb",
            "00000000000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c8",
            "00000000000000000000000000000000000000000000000000000000000f4240"
        );
        assert_eq!(hex::encode(&data), expected);
        // Same bytes as the typed sol! encoder used by `transfer`.
        let typed = crate::chains::evm::tx::erc20_transfer_calldata(
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
        );
        assert_eq!(data, typed);
    }

    #[test]
    fn encodes_arrays_tuples_bools_strings_negative_ints() {
        let f = parse_function("f(uint256[],(bool,string),int8,bytes)").unwrap();
        let data = encode_call(&f, &args(&["[1,2,3]", "(true, \"hi\")", "-5", "0xdead"])).unwrap();
        let decoded = f.abi_decode_input(&data[4..]).unwrap();
        let json: Vec<Value> = decoded.iter().map(to_json).collect();
        assert_eq!(
            Value::Array(json),
            serde_json::json!([["1", "2", "3"], ["true", "hi"], "-5", "0xdead"])
        );
    }

    #[test]
    fn decodes_uint_and_tuple_returns() {
        let f = parse_function("balanceOf(address)(uint256)").unwrap();
        let mut word = [0u8; 32];
        word[31] = 42;
        assert_eq!(decode_output(&f, &word).unwrap(), vec![Value::from("42")]);

        let g = parse_function("getReserves()(uint112,uint112,uint32)").unwrap();
        let mut data = vec![0u8; 96];
        data[31] = 1;
        data[63] = 2;
        data[95] = 3;
        assert_eq!(
            decode_output(&g, &data).unwrap(),
            vec![Value::from("1"), Value::from("2"), Value::from("3")]
        );
        assert!(decode_output(&g, &data[..10]).is_err());
    }

    #[test]
    fn signature_without_returns_has_no_outputs() {
        let f = parse_function("decimals()").unwrap();
        assert!(f.outputs.is_empty());
        let g = parse_function("function decimals() view returns (uint8)").unwrap();
        assert_eq!(g.outputs.len(), 1);
    }

    #[test]
    fn rejects_bad_signature_arity_and_values() {
        assert!(parse_function("not a sig(").is_err());
        let f = parse_function("transfer(address,uint256)").unwrap();
        assert!(encode_call(&f, &args(&["0x1"])).is_err());
        assert!(encode_call(&f, &args(&["0xnope", "1"])).is_err());
        assert!(
            encode_call(
                &f,
                &args(&["0x70997970C51812dc3A010C7d01b50e0d17dc79C8", "x"])
            )
            .is_err()
        );
    }

    const CHECKSUMMED: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const BAD_CHECKSUM: &str = "0x70997970c51812dc3A010C7d01b50e0d17dc79C8";

    /// Of (single-argument signature, argument) pairs that must be refused, the accepted ones.
    fn accepted(cases: &[(&str, &str)]) -> Vec<String> {
        cases
            .iter()
            .filter(|(sig, a)| encode_call(&parse_function(sig).unwrap(), &args(&[a])).is_ok())
            .map(|(sig, a)| format!("{sig} {a:?}"))
            .collect()
    }

    #[test]
    fn address_args_need_0x_and_a_valid_checksum_at_every_depth() {
        let no_prefix = &CHECKSUMMED[2..];
        let in_array = format!("[{CHECKSUMMED}, {BAD_CHECKSUM}]");
        let in_tuple = format!("({BAD_CHECKSUM}, 1)");
        let nested = format!("[({CHECKSUMMED}, [{no_prefix}])]");
        let fixed = format!("[{CHECKSUMMED},{BAD_CHECKSUM}]");
        let bad: Vec<(&str, &str)> = vec![
            ("f(address)", BAD_CHECKSUM),
            ("f(address)", no_prefix),
            ("f(address)", "vitalik.eth"),
            ("f(address[])", &in_array),
            ("f((address,uint256))", &in_tuple),
            ("f((address,address[])[])", &nested),
            ("f(address[2])", &fixed),
        ];
        let accepted = accepted(&bad);
        assert!(accepted.is_empty(), "accepted: {accepted:#?}");

        // Lowercase (no checksum claimed) and a correct checksum are fine, at any depth.
        let lower = CHECKSUMMED.to_lowercase();
        let f = parse_function("f(address,(address,address[]))").unwrap();
        let data = encode_call(
            &f,
            &args(&[
                &lower,
                &format!("({CHECKSUMMED}, [{lower}, {CHECKSUMMED}])"),
            ]),
        )
        .unwrap();
        let decoded: Vec<Value> = f
            .abi_decode_input(&data[4..])
            .unwrap()
            .iter()
            .map(to_json)
            .collect();
        assert_eq!(
            Value::Array(decoded),
            serde_json::json!([CHECKSUMMED, [CHECKSUMMED, [CHECKSUMMED, CHECKSUMMED]]])
        );
    }

    #[test]
    fn integer_args_are_plain_decimal_or_0x_hex() {
        let bad: Vec<(&str, &str)> = vec![
            ("f(uint256)", "100 ether"),
            ("f(uint256)", "1 gwei"),
            ("f(uint256)", "1e18"),
            ("f(uint256)", "4.2e1"),
            ("f(uint256)", "1.0"),
            ("f(uint256)", "+1"),
            ("f(uint256)", "-1"),
            ("f(uint256)", "0b101"),
            ("f(uint256)", "0o17"),
            ("f(uint256)", "0X10"),
            ("f(uint256)", "1_000"),
            ("f(uint256)", ""),
            ("f(uint256[])", "[1, 2 ether]"),
            ("f((uint8,int8))", "(1, 1e1)"),
            ("f(int256)", "-1 ether"),
            ("f(int256)", "+5"),
            ("f(int8)", "--5"),
        ];
        let accepted = accepted(&bad);
        assert!(accepted.is_empty(), "accepted: {accepted:#?}");

        let f = parse_function("f(uint256,uint8,int16,int8,uint256[])").unwrap();
        let data = encode_call(&f, &args(&["1000", "0xff", "-300", "0x7f", "[0x10, 17]"])).unwrap();
        let decoded: Vec<Value> = f
            .abi_decode_input(&data[4..])
            .unwrap()
            .iter()
            .map(to_json)
            .collect();
        assert_eq!(
            Value::Array(decoded),
            serde_json::json!(["1000", "255", "-300", "127", ["16", "17"]])
        );
        // Range is still enforced.
        assert!(encode_call(&parse_function("f(uint8)").unwrap(), &args(&["256"])).is_err());
        assert!(encode_call(&parse_function("f(int8)").unwrap(), &args(&["-129"])).is_err());
    }

    #[test]
    fn strings_bytes_and_bools_in_lists_still_work() {
        let f = parse_function("f(string[],(string,bool),bytes[],bytes2)").unwrap();
        let data = encode_call(
            &f,
            &args(&[
                "[\"a, b\", 'c]d', plain words]",
                "(\"x(y\", false)",
                "[0x, 0xbeef]",
                "0xabcd",
            ]),
        )
        .unwrap();
        let decoded: Vec<Value> = f
            .abi_decode_input(&data[4..])
            .unwrap()
            .iter()
            .map(to_json)
            .collect();
        assert_eq!(
            Value::Array(decoded),
            serde_json::json!([
                ["a, b", "c]d", "plain words"],
                ["x(y", "false"],
                ["0x", "0xbeef"],
                "0xabcd"
            ])
        );
        let g = parse_function("g(uint8[],uint8[2],(uint8,uint8))").unwrap();
        for bad in [
            ["[1,2,]", "[1,2]", "(1,2)"],
            ["[1,2]", "[1]", "(1,2)"],
            ["[1,2]", "[1,2]", "(1)"],
            ["[1,2]", "[1,2]", "(1,2,3)"],
            ["[1,2", "[1,2]", "(1,2)"],
            ["1,2", "[1,2]", "(1,2)"],
            ["[1,2]x", "[1,2]", "(1,2)"],
        ] {
            assert!(encode_call(&g, &args(&bad)).is_err(), "accepted {bad:?}");
        }
        let empty = encode_call(&g, &args(&["[]", "[ 1 , 2 ]", "( 3,4 )"])).unwrap();
        assert_eq!(
            Value::Array(
                g.abi_decode_input(&empty[4..])
                    .unwrap()
                    .iter()
                    .map(to_json)
                    .collect()
            ),
            serde_json::json!([[], ["1", "2"], ["3", "4"]])
        );
    }

    #[test]
    fn uint_and_int_aliases_get_the_canonical_selector() {
        // alloy-json-abi canonicalizes the aliases when it parses; pinned here because the
        // selector of `transfer(address,uint)` would otherwise be keccak of the alias.
        let f = parse_function("transfer(address,uint)").unwrap();
        assert_eq!(f.signature(), "transfer(address,uint256)");
        assert_eq!(hex::encode(f.selector()), "a9059cbb");
        let nested = parse_function("f((uint,int)[],uint[2],(bool,(int)))").unwrap();
        let canonical = parse_function("f((uint256,int256)[],uint256[2],(bool,(int256)))").unwrap();
        assert_eq!(nested.selector(), canonical.selector());
        // Aliases with no canonical form here are refused rather than hashed as written.
        for (sig, arg) in [("f(fixed)", "1"), ("f(ufixed)", "1"), ("g(byte)", "0x01")] {
            let f = parse_function(sig).unwrap();
            assert!(encode_call(&f, &args(&[arg])).is_err(), "{sig}");
        }
    }

    #[test]
    fn bytes_and_function_args_need_0x() {
        let (word, function) = ("ab".repeat(32), "ab".repeat(24));
        let bad: Vec<(&str, &str)> = vec![
            ("f(bytes)", "dead"),
            ("f(bytes2)", "abcd"),
            ("f(bytes32)", &word),
            ("f(function)", &function),
            ("f(bytes[])", "[0x01, 02]"),
            ("f((bytes1,uint8))", "(01, 1)"),
            ("f(bytes)", "0Xdead"),
        ];
        let accepted = accepted(&bad);
        assert!(accepted.is_empty(), "accepted: {accepted:#?}");
        let f = parse_function("f(bytes,bytes2,bytes1[])").unwrap();
        let data = encode_call(&f, &args(&["0xdead", "0xabcd", "[0x01, 0x02]"])).unwrap();
        let decoded: Vec<Value> = f
            .abi_decode_input(&data[4..])
            .unwrap()
            .iter()
            .map(to_json)
            .collect();
        assert_eq!(
            Value::Array(decoded),
            serde_json::json!(["0xdead", "0xabcd", ["0x01", "0x02"]])
        );
    }

    #[test]
    fn top_level_strings_are_verbatim_and_list_strings_unquote() {
        let f = parse_function("f(string,string,string[])").unwrap();
        let data = encode_call(
            &f,
            &args(&[
                "\"quoted\"",
                "  padded  ",
                "[\" kept spaces \", 'say \"hi\"', plain words, \"a, b\"]",
            ]),
        )
        .unwrap();
        let decoded: Vec<Value> = f
            .abi_decode_input(&data[4..])
            .unwrap()
            .iter()
            .map(to_json)
            .collect();
        assert_eq!(
            Value::Array(decoded),
            serde_json::json!([
                "\"quoted\"",
                "  padded  ",
                [" kept spaces ", "say \"hi\"", "plain words", "a, b"]
            ])
        );
        // A quoted list element must close with its own quote and not contain it.
        let g = parse_function("g(string[])").unwrap();
        for bad in ["[\"a\"b\"]", "[\"open]", "['x' y]"] {
            assert!(encode_call(&g, &args(&[bad])).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn calldata_parsing() {
        assert_eq!(parse_calldata("0xa9059cbb").unwrap().len(), 4);
        assert_eq!(parse_calldata("0x").unwrap().len(), 0);
        assert!(parse_calldata("a9059cbb").is_err());
        assert!(parse_calldata("0xzz").is_err());
    }
}

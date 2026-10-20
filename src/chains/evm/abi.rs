//! Function signatures for `call` / `invoke`: `name(argTypes)(returnTypes)`, e.g.
//! `balanceOf(address)(uint256)`. Arguments are CLI strings coerced to their ABI types.

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
        let value = ty.coerce_str(arg).map_err(|e| {
            WalletError::validation(format!(
                "argument {} ('{arg}') is not a valid {}: {e}",
                i + 1,
                param.ty
            ))
        })?;
        values.push(value);
    }
    func.abi_encode_input(&values)
        .map(Bytes::from)
        .map_err(|e| WalletError::validation(format!("cannot encode arguments: {e}")))
}

/// Decodes return data into JSON (scalars as strings, arrays/tuples as arrays).
pub fn decode_output(func: &Function, data: &[u8]) -> Result<Vec<Value>> {
    let values = func.abi_decode_output(data).map_err(|e| {
        WalletError::validation(format!(
            "return data (0x{}) does not match {}: {e}",
            hex::encode(data),
            func.signature_with_outputs()
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

    #[test]
    fn calldata_parsing() {
        assert_eq!(parse_calldata("0xa9059cbb").unwrap().len(), 4);
        assert_eq!(parse_calldata("0x").unwrap().len(), 0);
        assert!(parse_calldata("a9059cbb").is_err());
        assert!(parse_calldata("0xzz").is_err());
    }
}

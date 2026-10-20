//! Command-line surface (flat) and output rendering.

use crate::commands;
use crate::core::error::{ErrorCode, WalletError};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use std::ffi::OsString;

#[derive(Debug, Parser)]
#[command(
    name = "zwallet",
    version,
    about = "Local, login-free wallet CLI for agents. A human unlocks it for a chosen time; \
             a key-only daemon signs meanwhile."
)]
pub struct Cli {
    /// Machine-readable output. Failures print {"error","errorCode"} on stdout, exit 1.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List supported chains.
    Chains,
    /// Create the master seed (first run) or a new named key derived from it; the new key
    /// becomes active.
    Create(CreateArgs),
    /// List keys (* = active). Works while locked.
    List,
    /// Make NAME the active key. Works while locked.
    Change { name: String },
    /// Address of a key on a chain (default: ethereum). Works while locked.
    Address(AddressArgs),
    /// Addresses of a key on every chain. Works while locked.
    Addresses(KeyArg),
    /// Native or ERC-20 balance. Works while locked.
    Balance(BalanceArgs),
    /// Read-only contract call (eth_call). Works while locked.
    Call(CallArgs),
    /// State-changing contract transaction. Needs an unlocked wallet.
    Invoke(InvokeArgs),
    /// Native or ERC-20 transfer. Needs an unlocked wallet.
    Transfer(TransferArgs),
    /// EIP-191 personal_sign. Needs an unlocked wallet.
    SignMessage(SignMessageArgs),
    /// Wallet, lock state, and active key.
    Status,
    /// Print the recovery phrase. Interactive terminals only, after the password.
    Backup,
    /// Unlock signing for a duration: 30m, 24h, 365d, 1y (units s m h d w y).
    Unlock(UnlockArgs),
    /// Lock now: the key daemon wipes the key and exits.
    Lock,
    /// The key-holding daemon (started by `unlock`).
    #[command(hide = true)]
    Daemon,
}

#[derive(Debug, Args)]
pub struct KeyArg {
    /// Use this key instead of the active one.
    #[arg(long, value_name = "NAME")]
    pub key: Option<String>,
}

#[derive(Debug, Args)]
pub struct PasswordStdin {
    /// Read the password from one line of stdin instead of prompting on the terminal.
    #[arg(long)]
    pub password_stdin: bool,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Key name: 1-32 of a-z 0-9 _ -.
    pub name: String,
    #[command(flatten)]
    pub password: PasswordStdin,
    /// Also unlock for this duration after creating (e.g. 24h).
    #[arg(long, value_name = "DURATION")]
    pub unlock: Option<String>,
    /// Mnemonic length for a new master seed: 12, 15, 18, 21 or 24 (default 24).
    #[arg(long, value_name = "N")]
    pub words: Option<usize>,
}

#[derive(Debug, Args)]
pub struct AddressArgs {
    #[arg(long, default_value = "ethereum")]
    pub chain: String,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct BalanceArgs {
    #[arg(long)]
    pub chain: String,
    /// ERC-20 token contract (0x...). Omit for the native coin.
    #[arg(long)]
    pub token: Option<String>,
    /// RPC URL (overrides ZWALLET_RPC_<CHAIN> and the default).
    #[arg(long)]
    pub rpc: Option<String>,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct CallArgs {
    #[arg(long)]
    pub chain: String,
    /// Contract address.
    #[arg(long)]
    pub to: String,
    /// Function signature, e.g. "balanceOf(address)(uint256)". Return types are optional.
    #[arg(long)]
    pub sig: String,
    /// Function arguments in order (arrays as [1,2], tuples as (a,b)).
    #[arg(allow_negative_numbers = true)]
    pub args: Vec<String>,
    #[arg(long)]
    pub rpc: Option<String>,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct InvokeArgs {
    #[arg(long)]
    pub chain: String,
    /// Contract address.
    #[arg(long)]
    pub to: String,
    /// Function signature, e.g. "approve(address,uint256)".
    #[arg(long, required_unless_present = "data", conflicts_with = "data")]
    pub sig: Option<String>,
    /// Function arguments in order.
    #[arg(allow_negative_numbers = true)]
    pub args: Vec<String>,
    /// Raw 0x calldata instead of --sig and arguments.
    #[arg(long)]
    pub data: Option<String>,
    /// Native value to send, in whole units (e.g. 0.01).
    #[arg(long)]
    pub value: Option<String>,
    /// Build and sign from live chain state, but do not broadcast.
    #[arg(long)]
    pub dry_run: bool,
    /// Wait for the receipt after broadcasting.
    #[arg(long)]
    pub wait: bool,
    #[arg(long)]
    pub rpc: Option<String>,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct TransferArgs {
    #[arg(long)]
    pub chain: String,
    /// Recipient 0x address (ENS names are not supported).
    #[arg(long)]
    pub to: String,
    /// Amount in whole units (e.g. 0.5); exact, no floating point.
    #[arg(long)]
    pub amount: String,
    /// ERC-20 token contract. Omit for the native coin.
    #[arg(long)]
    pub token: Option<String>,
    /// Build and sign from live chain state, but do not broadcast.
    #[arg(long)]
    pub dry_run: bool,
    /// Wait for the receipt after broadcasting.
    #[arg(long)]
    pub wait: bool,
    #[arg(long)]
    pub rpc: Option<String>,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct SignMessageArgs {
    #[arg(long, default_value = "ethereum")]
    pub chain: String,
    /// The message, signed as UTF-8 bytes.
    #[arg(long, allow_hyphen_values = true)]
    pub message: String,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct UnlockArgs {
    /// How long to stay unlocked: 30m, 24h, 365d, 1y (units s m h d w y).
    pub duration: String,
    #[command(flatten)]
    pub password: PasswordStdin,
}

/// A command's result: JSON, plus optional human text (generic rendering otherwise).
pub struct Out {
    pub value: Value,
    pub text: Option<String>,
}

impl Out {
    pub fn json(value: Value) -> Self {
        Self { value, text: None }
    }

    pub fn with_text(value: Value, text: String) -> Self {
        Self {
            value,
            text: Some(text),
        }
    }

    pub fn silent() -> Self {
        Self {
            value: Value::Null,
            text: Some(String::new()),
        }
    }
}

/// Process entry: parse, run, print. Returns the exit code.
pub fn main_entry() -> i32 {
    let args: Vec<OsString> = std::env::args_os().collect();
    let json = args.iter().skip(1).any(|a| a == "--json");
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    0
                }
                _ if json => {
                    let rendered = e.render().to_string();
                    let first = rendered
                        .lines()
                        .next()
                        .unwrap_or("invalid arguments")
                        .trim_start_matches("error: ")
                        .to_string();
                    print_error(true, &WalletError::new(ErrorCode::ValidationError, first));
                    1
                }
                _ => {
                    let _ = e.print();
                    1
                }
            };
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            print_error(
                json,
                &WalletError::validation(format!("cannot start runtime: {e}")),
            );
            return 1;
        }
    };
    match runtime.block_on(commands::run(cli)) {
        Ok(out) => {
            print_out(json, &out);
            0
        }
        Err(e) => {
            print_error(json, &e);
            1
        }
    }
}

fn print_error(json: bool, e: &WalletError) {
    if json {
        println!("{}", e.to_json());
    } else {
        eprintln!("error: {} [{}]", e.message, e.code);
    }
}

fn print_out(json: bool, out: &Out) {
    if json {
        if !out.value.is_null() {
            println!(
                "{}",
                serde_json::to_string_pretty(&out.value).unwrap_or_default()
            );
        }
        return;
    }
    match &out.text {
        Some(t) if t.is_empty() => {}
        Some(t) => println!("{t}"),
        None => println!("{}", render_human(&out.value)),
    }
}

/// `key: value` lines for an object; nested values as compact JSON.
pub fn render_human(value: &Value) -> String {
    match value {
        Value::Object(map) => map
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| format!("{k}: {}", scalar(v)))
            .collect::<Vec<_>>()
            .join("\n"),
        other => scalar(other),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn call_args_accept_negative_numbers_and_trailing_json() {
        let cli = Cli::try_parse_from([
            "zwallet",
            "call",
            "--chain",
            "ethereum",
            "--to",
            "0x00",
            "--sig",
            "f(int8,uint8)",
            "-5",
            "7",
            "--json",
        ])
        .unwrap();
        assert!(cli.json);
        match cli.command {
            Command::Call(a) => assert_eq!(a.args, vec!["-5", "7"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn invoke_needs_sig_or_data_not_both() {
        let base = ["zwallet", "invoke", "--chain", "base", "--to", "0x00"];
        assert!(Cli::try_parse_from(base).is_err());
        let mut both = base.to_vec();
        both.extend(["--sig", "f()", "--data", "0x"]);
        assert!(Cli::try_parse_from(both).is_err());
        let mut data = base.to_vec();
        data.extend(["--data", "0xd0e30db0", "--value", "0.1"]);
        assert!(Cli::try_parse_from(data).is_ok());
    }

    #[test]
    fn unlock_duration_is_positional() {
        let cli = Cli::try_parse_from(["zwallet", "unlock", "24h"]).unwrap();
        match cli.command {
            Command::Unlock(a) => assert_eq!(a.duration, "24h"),
            other => panic!("{other:?}"),
        }
        assert!(Cli::try_parse_from(["zwallet", "unlock"]).is_err());
    }
}

//! Command-line surface (flat) and output rendering.

use crate::commands;
use crate::core::error::{ErrorCode, WalletError};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use std::ffi::OsString;
use std::sync::LazyLock;

/// [`crate::version_line`] without its `zwallet ` prefix (clap adds the name).
fn version_text() -> &'static str {
    static TEXT: LazyLock<String> = LazyLock::new(|| {
        let line = crate::version_line();
        line.strip_prefix("zwallet ").unwrap_or(&line).to_string()
    });
    TEXT.as_str()
}

#[derive(Debug, Parser)]
#[command(
    name = "zwallet",
    version = version_text(),
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
    List(VerifyArgs),
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
    /// Look a transaction up by hash: mined (with its receipt), pending, or not found. Works
    /// while locked, and without a wallet.
    Tx(TxArgs),
    /// Wallet, lock state, and active key.
    Status,
    /// Print the recovery phrase. Interactive terminals only, after the password.
    Backup,
    /// Unlock signing for a duration: 30m, 24h, 365d, 1y (units s m h d w y).
    Unlock(UnlockArgs),
    /// Lock now: stops every zwallet key daemon of this user, whatever home it serves. Reads no
    /// home.
    Lock,
    /// The signed whitelist: the only payees and contracts the key daemon signs for. `list`
    /// works while locked; every change asks for the password.
    #[command(subcommand)]
    Whitelist(WhitelistCommand),
    /// The key-holding daemon (started by `unlock`).
    #[command(hide = true)]
    Daemon {
        /// Absolute wallet home (set by `unlock`; the process table shows it).
        #[arg(long)]
        home: std::path::PathBuf,
    },
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
    /// With --unlock: see `unlock --allow-ptrace-scope-0`.
    #[arg(long, requires = "unlock")]
    pub allow_ptrace_scope_0: bool,
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
    #[command(flatten)]
    pub verify: VerifyArgs,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Decrypt the wallet here with the password and check the address against the seed; only
    /// this reports `verified: true`. Run it before funding an address.
    #[arg(long)]
    pub verify: bool,
    /// With --verify: read the password from one line of stdin instead of prompting.
    #[arg(long, requires = "verify")]
    pub password_stdin: bool,
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
    /// Contract address, or @NICKNAME from the whitelist.
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
    #[command(flatten)]
    pub send: SendArgs,
    #[arg(long)]
    pub rpc: Option<String>,
    #[command(flatten)]
    pub key: KeyArg,
}

/// Options shared by the commands that sign and broadcast a transaction.
#[derive(Debug, Args)]
pub struct SendArgs {
    /// Build and sign from live chain state, but do not broadcast.
    #[arg(long)]
    pub dry_run: bool,
    /// Wait for the receipt after broadcasting (the send lock is held meanwhile).
    #[arg(long)]
    pub wait: bool,
    /// With --wait: how long to wait for the receipt, e.g. 30s, 5m (default 90s).
    #[arg(long, value_name = "DURATION", requires = "wait", value_parser = duration_arg)]
    pub wait_timeout: Option<std::time::Duration>,
    /// Use this nonce instead of the next free one. Below the next free one it replaces the
    /// pending transaction there, which --replace must name; a retry after BROADCAST_UNCERTAIN
    /// passes --nonce N --replace <txHash> so that it cannot become a second send.
    #[arg(long, value_name = "N")]
    pub nonce: Option<u64>,
    /// With --nonce: pay this many times the fee estimate (and a replaced transaction's fees),
    /// from 1.1 to 3 (default 1.25 when replacing), so that nodes take it over the pending
    /// transaction. The fee ceiling still applies.
    #[arg(long, value_name = "FACTOR", requires = "nonce")]
    pub bump: Option<String>,
    /// With --nonce: the txHash of the pending transaction being replaced (required below the
    /// next free nonce). Its fees are read from the node, and the replacement also pays at least
    /// --bump times them.
    #[arg(long, value_name = "TXHASH", requires = "nonce")]
    pub replace: Option<String>,
    /// Send even though a transaction from this key is still pending. Only for a different
    /// payment: a re-run of a send that was cut off would land as a second payment.
    #[arg(long, conflicts_with = "nonce")]
    pub allow_pending: bool,
    /// Send even though the send journal has the same send (recipient, value, token) from the
    /// last 10 minutes. Only for a real second payment: a re-run of a send that went through
    /// would pay twice.
    #[arg(long, conflicts_with = "nonce")]
    pub allow_duplicate: bool,
    /// Allow a price per gas up to this many gwei, replacing the automatic ceiling on fees the
    /// RPC suggests (raise it only when fees really are that high).
    #[arg(long, value_name = "GWEI")]
    pub max_fee_gwei: Option<String>,
}

#[derive(Debug, Args)]
pub struct TransferArgs {
    #[arg(long)]
    pub chain: String,
    /// Recipient: a 0x address, or @NICKNAME from the whitelist (ENS names are not supported).
    #[arg(long)]
    pub to: String,
    /// Amount in whole units (e.g. 0.5); exact, no floating point.
    #[arg(long)]
    pub amount: String,
    /// ERC-20 token contract. Omit for the native coin.
    #[arg(long)]
    pub token: Option<String>,
    /// Allow a recipient that is almost always a mistake: the zero address, or the token
    /// contract itself.
    #[arg(long)]
    pub force: bool,
    #[command(flatten)]
    pub send: SendArgs,
    #[arg(long)]
    pub rpc: Option<String>,
    #[command(flatten)]
    pub key: KeyArg,
}

#[derive(Debug, Args)]
pub struct TxArgs {
    #[arg(long)]
    pub chain: String,
    /// The transaction hash (0x and 64 hex digits).
    #[arg(value_name = "TXHASH")]
    pub tx_hash: String,
    /// RPC URL (overrides ZWALLET_RPC_<CHAIN> and the default).
    #[arg(long)]
    pub rpc: Option<String>,
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
    /// Linux only: start the key daemon even though kernel.yama.ptrace_scope is 0 or missing,
    /// i.e. any same-user process can attach to it during start-up.
    #[arg(long)]
    pub allow_ptrace_scope_0: bool,
}

#[derive(Debug, Subcommand)]
pub enum WhitelistCommand {
    /// The list: generation, entries and policy. Works while locked (`verified: false`).
    List(WhitelistListArgs),
    /// Allow payments to ADDRESS as NICKNAME (with --contract: calls of its --selectors).
    Add(WhitelistAddArgs),
    /// Remove the entry NICKNAME.
    Remove(WhitelistRemoveArgs),
    /// Set CHAIN's fee cap, in its coin: the most gasLimit × max price may cost.
    FeeCap(WhitelistFeeCapArgs),
    /// Let `sign-message` sign (on) or refuse it (off, the default).
    AllowMessageSigning(WhitelistToggleArgs),
    /// A new whitelist key and an empty list, for a lost or damaged whitelist.key.
    Reset(WhitelistEditArgs),
}

#[derive(Debug, Args)]
pub struct WhitelistListArgs {
    /// Check the list's signature against wallet.json's seal with the password; only this
    /// reports `verified: true`.
    #[arg(long)]
    pub verify: bool,
    /// With --verify: read the password from one line of stdin instead of prompting.
    #[arg(long, requires = "verify")]
    pub password_stdin: bool,
}

/// What every whitelist change takes. A change stops every zwallet daemon of this user first and
/// leaves the wallet locked, unless --unlock starts a new session.
#[derive(Debug, Args)]
pub struct WhitelistEditArgs {
    #[command(flatten)]
    pub password: PasswordStdin,
    /// After the change, unlock for this duration (e.g. 24h) with the password just typed.
    #[arg(long, value_name = "DURATION")]
    pub unlock: Option<String>,
    /// With --unlock: see `unlock --allow-ptrace-scope-0`.
    #[arg(long, requires = "unlock")]
    pub allow_ptrace_scope_0: bool,
}

#[derive(Debug, Args)]
pub struct WhitelistAddArgs {
    /// 1-32 of a-z 0-9 _ -, starting with a letter or digit; `@NICKNAME` then works as
    /// `transfer --to` and `invoke --to`.
    pub nickname: String,
    /// The 0x address (mixed case must be a valid EIP-55 checksum).
    pub address: String,
    /// Chain keys the entry applies on, comma-separated, or "*" (quoted) for every chain (not
    /// for a contract). Default: ethereum.
    #[arg(
        long,
        visible_alias = "chain",
        value_delimiter = ',',
        value_name = "CHAINS"
    )]
    pub chains: Vec<String>,
    /// A contract entry: only calls with one of --selectors, never a plain payment.
    #[arg(long, requires = "selectors")]
    pub contract: bool,
    /// With --contract: the 4-byte function selectors it may be called with (0x12345678,…).
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "SELECTORS",
        requires = "contract"
    )]
    pub selectors: Vec<String>,
    #[command(flatten)]
    pub edit: WhitelistEditArgs,
}

#[derive(Debug, Args)]
pub struct WhitelistRemoveArgs {
    pub nickname: String,
    #[command(flatten)]
    pub edit: WhitelistEditArgs,
}

#[derive(Debug, Args)]
pub struct WhitelistFeeCapArgs {
    pub chain: String,
    /// The cap in the chain's coin (e.g. 0.001), exact; more than zero.
    pub amount: String,
    #[command(flatten)]
    pub edit: WhitelistEditArgs,
}

#[derive(Debug, Args)]
pub struct WhitelistToggleArgs {
    pub setting: Toggle,
    #[command(flatten)]
    pub edit: WhitelistEditArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Toggle {
    On,
    Off,
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
    // First, for every command: no core dump, and on Linux non-dumpable (see the function).
    crate::core::password::disable_core_dumps();
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

/// A `--wait-timeout` value: a [`crate::core::time::parse_duration`] duration.
fn duration_arg(s: &str) -> Result<std::time::Duration, String> {
    crate::core::time::parse_duration(s).map_err(|e| e.message)
}

/// Error details a human needs to look a send up, printed under the message.
const HUMAN_DETAILS: &[&str] = &[
    "txHash",
    "refusedTxHash",
    "minedTxHash",
    "success",
    "nonce",
    "from",
    "journal",
    // Whitelist refusals: what the refusal is about, and the command a human runs to allow it.
    "counterparty",
    "chain",
    "command",
];

fn print_error(json: bool, e: &WalletError) {
    if json {
        println!("{}", e.to_json());
    } else {
        eprintln!("{}", human_error(e));
    }
}

/// `error: <message> [CODE]`, then one `<key>: <value>` line per [`HUMAN_DETAILS`] field.
fn human_error(e: &WalletError) -> String {
    let mut text = format!("error: {} [{}]", e.message, e.code);
    for key in HUMAN_DETAILS {
        if let Some(v) = e.details.get(*key) {
            text.push_str(&format!("\n{key}: {}", scalar(v)));
        }
    }
    text
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
    fn version_flag_prints_the_version_line() {
        for flag in ["--version", "-V"] {
            let err = Cli::try_parse_from(["zwallet", flag]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::DisplayVersion, "{flag}");
            assert_eq!(err.to_string().trim_end(), crate::version_line(), "{flag}");
        }
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
    fn send_flags_parse_on_transfer_and_invoke() {
        let cli = Cli::try_parse_from([
            "zwallet",
            "transfer",
            "--chain",
            "base",
            "--to",
            "0x00",
            "--amount",
            "1",
            "--nonce",
            "12",
            "--max-fee-gwei",
            "0.5",
            "--force",
            "--wait",
        ])
        .unwrap();
        match cli.command {
            Command::Transfer(a) => {
                assert_eq!(a.send.nonce, Some(12));
                assert_eq!(a.send.max_fee_gwei.as_deref(), Some("0.5"));
                assert!(a.force && a.send.wait && !a.send.dry_run);
            }
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from([
            "zwallet",
            "invoke",
            "--chain",
            "base",
            "--to",
            "0x00",
            "--data",
            "0x",
            "--nonce",
            "3",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Invoke(a) => assert_eq!((a.send.nonce, a.send.dry_run), (Some(3), true)),
            other => panic!("{other:?}"),
        }
        let cli = Cli::try_parse_from([
            "zwallet",
            "invoke",
            "--chain",
            "base",
            "--to",
            "0x00",
            "--data",
            "0x",
            "--nonce",
            "3",
            "--bump",
            "1.5",
            "--replace",
            "0xab",
        ])
        .unwrap();
        match cli.command {
            Command::Invoke(a) => assert_eq!(
                (a.send.bump.as_deref(), a.send.replace.as_deref()),
                (Some("1.5"), Some("0xab"))
            ),
            other => panic!("{other:?}"),
        }
        // A negative or fractional nonce is a parse error; --force is transfer-only; --bump and
        // --replace need --nonce; --wait-timeout needs --wait and a duration; --allow-pending
        // is for a send without --nonce.
        let bad = [
            "zwallet", "invoke", "--chain", "base", "--to", "0x00", "--data", "0x",
        ];
        for extra in [
            &["--nonce", "-1"][..],
            &["--nonce", "1.5"],
            &["--force", "--dry-run"],
            &["--bump", "2"],
            &["--replace", "0xab"],
            &["--wait-timeout", "30s"],
            &["--wait", "--wait-timeout", "30"],
            &["--wait", "--wait-timeout", "0s"],
            &["--allow-pending", "--nonce", "3"],
            &["--allow-duplicate", "--nonce", "3"],
        ] {
            let mut args = bad.to_vec();
            args.extend(extra);
            assert!(Cli::try_parse_from(args).is_err(), "{extra:?}");
        }
        let mut args = bad.to_vec();
        args.extend([
            "--wait",
            "--wait-timeout",
            "2m",
            "--allow-pending",
            "--allow-duplicate",
        ]);
        match Cli::try_parse_from(args).unwrap().command {
            Command::Invoke(a) => {
                assert_eq!(
                    a.send.wait_timeout,
                    Some(std::time::Duration::from_secs(120))
                );
                assert!(a.send.allow_pending && a.send.allow_duplicate);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn human_errors_show_the_send_details() {
        let e = WalletError::new(ErrorCode::BroadcastUncertain, "maybe sent")
            .with_detail("refusedTxHash", "0xdead")
            .with_detail("nonce", 7)
            .with_detail("from", "0xF00")
            .with_detail("journal", "/h/sends-1-0xf00.jsonl")
            .with_detail("pendingSends", serde_json::json!([]));
        assert_eq!(
            human_error(&e),
            "error: maybe sent [BROADCAST_UNCERTAIN]\nrefusedTxHash: 0xdead\nnonce: 7\nfrom: \
             0xF00\njournal: /h/sends-1-0xf00.jsonl"
        );
        // A mined nonce: the transaction there and its receipt's outcome.
        let mined = WalletError::validation("already mined")
            .with_detail("nonce", 4)
            .with_detail("minedTxHash", "0xab")
            .with_detail("success", false);
        assert_eq!(
            human_error(&mined),
            "error: already mined [VALIDATION_ERROR]\nminedTxHash: 0xab\nsuccess: false\nnonce: 4"
        );
        let plain = WalletError::validation("bad");
        assert_eq!(human_error(&plain), "error: bad [VALIDATION_ERROR]");
    }

    #[test]
    fn balance_chain_is_optional_but_token_and_rpc_need_it() {
        let parse = |args: &[&str]| {
            let mut all = vec!["zwallet", "balance"];
            all.extend_from_slice(args);
            Cli::try_parse_from(all).map(|c| c.command)
        };
        // No --chain: every network.
        for ok in [&[][..], &["--key", "main", "--json"]] {
            assert!(matches!(parse(ok), Ok(Command::Balance(_))), "{ok:?}");
        }
        // One chain, with or without a token contract and an RPC URL.
        assert!(matches!(
            parse(&["--chain", "base", "--token", "0xab", "--rpc", "http://x"]),
            Ok(Command::Balance(_))
        ));
        // A token contract and an RPC URL belong to one chain.
        for bad in [&["--token", "0xab"][..], &["--rpc", "http://x"]] {
            let e = parse(bad).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::MissingRequiredArgument, "{bad:?}");
            assert!(e.to_string().contains("--chain <CHAIN>"), "{e}");
        }
    }

    #[test]
    fn balance_help_says_no_chain_means_every_network() {
        let mut cli = Cli::command();
        let balance = cli.find_subcommand_mut("balance").unwrap();
        let help = balance.render_help().to_string();
        assert!(help.contains("every network"), "{help}");
        let long = balance.render_long_help().to_string();
        assert!(long.contains("Omit to check every network"), "{long}");
    }

    #[test]
    fn tx_takes_a_positional_hash() {
        let cli =
            Cli::try_parse_from(["zwallet", "tx", "--chain", "base", "0xab", "--json"]).unwrap();
        assert!(cli.json);
        match cli.command {
            Command::Tx(a) => assert_eq!((a.chain.as_str(), a.tx_hash.as_str()), ("base", "0xab")),
            other => panic!("{other:?}"),
        }
        assert!(Cli::try_parse_from(["zwallet", "tx", "--chain", "base"]).is_err());
        assert!(Cli::try_parse_from(["zwallet", "tx", "0xab"]).is_err());
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

    #[test]
    fn whitelist_verbs_parse_the_forms_the_errors_name() {
        let parse = |args: &[&str]| {
            let mut all = vec!["zwallet", "whitelist"];
            all.extend_from_slice(args);
            Cli::try_parse_from(all).map(|c| c.command)
        };
        match parse(&["add", "alice", "0xab", "--chains", "base,ethereum"]).unwrap() {
            Command::Whitelist(WhitelistCommand::Add(a)) => {
                assert_eq!((a.nickname.as_str(), a.address.as_str()), ("alice", "0xab"));
                assert_eq!(a.chains, ["base", "ethereum"]);
                assert!(!a.contract && a.selectors.is_empty());
            }
            other => panic!("{other:?}"),
        }
        match parse(&[
            "add",
            "router",
            "0xab",
            "--chain",
            "base",
            "--contract",
            "--selectors",
            "0x12345678,0xabcdef01",
            "--password-stdin",
        ])
        .unwrap()
        {
            Command::Whitelist(WhitelistCommand::Add(a)) => {
                assert_eq!(a.chains, ["base"]);
                assert_eq!(a.selectors, ["0x12345678", "0xabcdef01"]);
                assert!(a.contract && a.edit.password.password_stdin);
            }
            other => panic!("{other:?}"),
        }
        match parse(&["fee-cap", "base", "0.001"]).unwrap() {
            Command::Whitelist(WhitelistCommand::FeeCap(a)) => {
                assert_eq!((a.chain.as_str(), a.amount.as_str()), ("base", "0.001"));
            }
            other => panic!("{other:?}"),
        }
        match parse(&["allow-message-signing", "on"]).unwrap() {
            Command::Whitelist(WhitelistCommand::AllowMessageSigning(a)) => {
                assert_eq!(a.setting, Toggle::On);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse(&["list", "--verify", "--password-stdin"]).unwrap(),
            Command::Whitelist(WhitelistCommand::List(a)) if a.verify && a.password_stdin
        ));
        match parse(&["reset", "--unlock", "24h", "--allow-ptrace-scope-0"]).unwrap() {
            Command::Whitelist(WhitelistCommand::Reset(a)) => {
                assert_eq!(a.unlock.as_deref(), Some("24h"));
                assert!(a.allow_ptrace_scope_0);
            }
            other => panic!("{other:?}"),
        }
        for bad in [
            &["reset", "--allow-ptrace-scope-0"][..],
            &["add", "r", "0xab", "--contract"],
            &["add", "r", "0xab", "--selectors", "0x12345678"],
            &["allow-message-signing", "yes"],
            &["list", "--password-stdin"],
            &["remove"],
            &["fee-cap", "base"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}

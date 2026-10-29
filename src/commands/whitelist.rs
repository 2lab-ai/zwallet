//! Whitelist CLI adapter: parsing flags, prompting and rendering. All mutation validation,
//! summaries, authentication and edit ordering live in the shared wallet boundary.
use crate::cli::{Out, Toggle, WhitelistCommand, WhitelistEditArgs, WhitelistListArgs};
use crate::core::error::Result;
use crate::core::password::read_password;
use crate::core::whitelist::EntryKind;
use crate::wallet::{self, Change, WhitelistUnlock};
use std::path::Path;

pub(crate) use crate::wallet::whitelist::{DryRunCheck, dry_run_check, recipient};

pub(super) async fn run(home: &Path, cmd: WhitelistCommand) -> Result<Out> {
    let (change, opts) = match cmd {
        WhitelistCommand::List(a) => return list(home, a),
        WhitelistCommand::Add(a) => {
            let chain_keys: Vec<&str> = a.chains.iter().map(String::as_str).collect();
            let selectors: Vec<&str> = a.selectors.iter().map(String::as_str).collect();
            let kind = if a.contract {
                EntryKind::Contract
            } else {
                EntryKind::Address
            };
            (
                Change::add(&a.nickname, &a.address, &chain_keys, kind, &selectors)?,
                a.edit,
            )
        }
        WhitelistCommand::Remove(a) => (Change::remove(&a.nickname)?, a.edit),
        WhitelistCommand::FeeCap(a) => (Change::fee_cap(&a.chain, &a.amount)?, a.edit),
        WhitelistCommand::NativeCap(a) => (Change::native_cap(&a.chain, &a.amount)?, a.edit),
        WhitelistCommand::TokenCap(a) => (
            Change::token_cap(&a.chain, &a.token, &a.amount_raw)?,
            a.edit,
        ),
        WhitelistCommand::CalldataCap(a) => (Change::calldata_cap(&a.chain, &a.bytes)?, a.edit),
        WhitelistCommand::AllowMessageSigning(a) => {
            (Change::message_signing(a.setting == Toggle::On), a.edit)
        }
        WhitelistCommand::Reset(a) => (Change::reset(), a),
    };
    edit(home, change, opts).await
}

async fn edit(home: &Path, change: Change, opts: WhitelistEditArgs) -> Result<Out> {
    let unlock = opts.unlock.map(|duration| WhitelistUnlock {
        duration,
        allow_ptrace_scope_0: opts.allow_ptrace_scope_0,
    });
    let plan = wallet::whitelist_plan(home, change, unlock)?;
    let password = if opts.password.password_stdin {
        crate::core::terminal::eprint_line(plan.summary());
        read_password(true, "", false)?
    } else {
        read_password(
            false,
            &format!("{}\nWallet password: ", plan.summary()),
            false,
        )?
    };
    let outcome = wallet::whitelist_apply(plan, password).await?;
    Ok(Out::with_text(outcome.output, outcome.text))
}

fn list(home: &Path, a: WhitelistListArgs) -> Result<Out> {
    crate::core::store::read_wallet_file(home)?;
    let opened = if a.verify {
        let password = read_password(a.password_stdin, "Wallet password: ", false)?;
        let opened = wallet::open_with_password(home, &password, Some(&[]))?;
        drop(password);
        Some(opened)
    } else {
        None
    };
    let view = wallet::whitelist_view(home, opened.as_ref().map(|o| &o.anchor))?;
    Ok(Out::with_text(view.output, view.text))
}

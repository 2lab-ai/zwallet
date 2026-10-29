//! Shared verified whitelist state at the family signing boundary.

use crate::core::whitelist::Whitelist;

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
    /// This wallet has no whitelist yet (wallet.json seals no anchor). The family policy decides whether an exception applies (only EVM native self-transfers). Refusals name the edit
    /// that would allow the request; that first edit creates the list.
    NoList,
    /// There is a list or an anchor, but nothing that can be trusted, and why: it does not
    /// verify, cannot be read (in time), or the sealed anchor is malformed. Nothing is signed,
    /// self-transfers included. Refusals name no edit (one would rebuild the list around whatever
    /// the request asks for); they point to [`VERIFY_COMMAND`].
    Unusable(String),
}

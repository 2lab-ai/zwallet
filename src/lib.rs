//! zwallet: a local, login-free wallet CLI for agents.
//!
//! Layout: `core` (chain-agnostic custody, protocol, amounts), `chains` (registry + family
//! boundary; `chains::evm` today), `daemon` (the key holder), `cli` + `commands` (one-shot CLI).

pub mod chains;
pub mod cli;
pub mod commands;
pub mod core;
pub mod daemon;
mod qr;
pub mod tui;
pub mod wallet;

/// `zwallet <ver> (<channel> <rev12>)` — the `zwallet --version` line. Channel and build id come
/// from `ZWALLET_BUILD_CHANNEL` / `ZWALLET_BUILD_ID` at compile time (`dev` / `unknown`
/// otherwise); the build id is a commit sha, of which the first 12 chars are kept.
pub fn version_line() -> String {
    let channel = option_env!("ZWALLET_BUILD_CHANNEL").unwrap_or("dev");
    let rev12: String = option_env!("ZWALLET_BUILD_ID")
        .unwrap_or("unknown")
        .chars()
        .take(12)
        .collect();
    format!("zwallet {} ({channel} {rev12})", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_line_shape() {
        let line = super::version_line();
        let prefix = concat!("zwallet ", env!("CARGO_PKG_VERSION"), " (");
        let inner = line
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap_or_else(|| panic!("{line}"));
        let parts: Vec<&str> = inner.split(' ').collect();
        assert_eq!(parts.len(), 2, "{line}");
        let (channel, rev12) = (parts[0], parts[1]);
        assert!(!channel.is_empty(), "{line}");
        assert!(!rev12.is_empty() && rev12.chars().count() <= 12, "{line}");
        if option_env!("ZWALLET_BUILD_CHANNEL").is_none() {
            assert_eq!(channel, "dev", "{line}");
        }
    }
}

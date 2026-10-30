//! One action table for menus, hints and dispatch. Coverage row numbers remain stable when
//! CLI-only rows are skipped. An action is disabled unless its handler is explicitly installed;
//! listing a future action never grants a signing capability.

use super::form::{Edit, Field, Input, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Keys,
    Wallet,
    Send,
    Receive,
    Whitelist,
    Tools,
    Session,
}
impl Tab {
    pub const ALL: [Self; 7] = [
        Self::Keys,
        Self::Wallet,
        Self::Send,
        Self::Receive,
        Self::Whitelist,
        Self::Tools,
        Self::Session,
    ];
    pub const fn number(self) -> u8 {
        self as u8 + 1
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keys => "keys",
            Self::Wallet => "wallet",
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Whitelist => "whitelist",
            Self::Tools => "tools",
            Self::Session => "session",
        }
    }
    pub fn from_number(number: u8) -> Option<Self> {
        number
            .checked_sub(1)
            .and_then(|n| Self::ALL.get(n as usize).copied())
    }
    pub fn next(self) -> Self {
        Self::ALL[self.number() as usize % Self::ALL.len()]
    }
    pub fn previous(self) -> Self {
        Self::ALL[(self.number() as usize + Self::ALL.len() - 2) % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionId {
    List,
    VerifyKeys,
    CreateFirst,
    CreateKey,
    CreateUnlock,
    SetActive,
    Unlock,
    PtraceFlag,
    Lock,
    Status,
    Backup,
    Address,
    Repair,
    Qr,
    Addresses,
    AllBalances,
    Balance,
    TokenBalance,
    Chains,
    Rpc,
    Transfer,
    TokenTransfer,
    DryRun,
    Wait,
    AllowPending,
    AllowDuplicate,
    GasCap,
    SpeedUp,
    Cancel,
    Retry,
    Tx,
    Call,
    Invoke,
    RawInvoke,
    InvokeOptions,
    SignMessage,
    WhitelistList,
    WhitelistVerify,
    WhitelistAdd,
    WhitelistContract,
    WhitelistRemove,
    FeeCap,
    MessagePolicy,
    Reset,
    WhitelistUnlock,
    Startup,
    SelectKey,
    Version,
    TronAddress,
    TronBalance,
    TronTransfer,
    TronTx,
    NativeCap,
    TokenCap,
    CalldataCap,
    DuplicateWindow,
    TaskCreate,
    TaskList,
    TaskReport,
    TaskAttribution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    Always,
    Wallet,
    FirstWallet,
    OpenKey,
    Verified,
    Signing,
    Pending,
    PendingSigning,
    Retryable,
    StartFlag,
    StartupInfo,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Any,
    Evm,
    Tron,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: ActionId,
    /// Number in 03-tui.md's coverage matrix, not an array index.
    pub row: u8,
    pub alias: &'static str,
    pub key: &'static str,
    pub tab: Tab,
    pub label: &'static str,
    pub requirement: Requirement,
    pub family: Family,
}

/// The state of the actual handlers. No default set of implementations is implied. The caller
/// lists only actions wired to real handlers; future rows stay visible with a disabled reason.
#[derive(Default)]
pub struct Context<'a> {
    pub implemented: &'a [ActionId],
    pub has_wallet: bool,
    pub has_open_key: bool,
    pub verified: bool,
    pub unlocked: bool,
    pub pending_send: bool,
    pub retryable_send: bool,
    pub is_tron: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Ready,
    Disabled(&'static str),
}
impl Action {
    pub fn availability(&self, context: &Context<'_>) -> Availability {
        use Availability::{Disabled, Ready};
        match self.requirement {
            Requirement::StartFlag => {
                return Disabled(
                    "start flag: zwallet tui --allow-ptrace-scope-0; no key can lower this protection",
                );
            }
            Requirement::StartupInfo => {
                return Disabled("start flags: --chain and --warm; use c or u in the TUI");
            }
            _ => {}
        }
        if !context.implemented.contains(&self.id) {
            return Disabled("in a later version");
        }
        if self.family == Family::Evm && context.is_tron {
            return Disabled("not supported on Tron");
        }
        if self.family == Family::Tron && !context.is_tron {
            return Disabled("select Tron with c");
        }
        match self.requirement {
            Requirement::FirstWallet if context.has_wallet => {
                Disabled("wallet exists; use create another key")
            }
            Requirement::Wallet
            | Requirement::OpenKey
            | Requirement::Verified
            | Requirement::Signing
            | Requirement::PendingSigning
            | Requirement::Retryable
                if !context.has_wallet =>
            {
                Disabled("needs: a wallet")
            }
            Requirement::OpenKey
            | Requirement::Verified
            | Requirement::Signing
            | Requirement::PendingSigning
            | Requirement::Retryable
                if !context.has_open_key =>
            {
                Disabled("needs: an open key")
            }
            Requirement::Verified
            | Requirement::Signing
            | Requirement::PendingSigning
            | Requirement::Retryable
                if !context.verified =>
            {
                Disabled("needs: V")
            }
            Requirement::Signing | Requirement::PendingSigning | Requirement::Retryable
                if !context.unlocked =>
            {
                Disabled("needs: a warm-up")
            }
            Requirement::Pending | Requirement::PendingSigning if !context.pending_send => {
                Disabled("needs: a pending send")
            }
            Requirement::Retryable if !context.retryable_send => {
                Disabled("needs: this session's uncertain send")
            }
            _ => Ready,
        }
    }
}

macro_rules! action {
    ($id:ident, $row:literal, $alias:literal, $key:literal, $tab:ident, $label:literal, $requirement:ident, $family:ident) => {
        Action {
            id: ActionId::$id,
            row: $row,
            alias: $alias,
            key: $key,
            tab: Tab::$tab,
            label: $label,
            requirement: Requirement::$requirement,
            family: Family::$family,
        }
    };
}

/// Matrix rows 1-52 followed by signed limits, duplicate window, and native task controls (60-67).
#[rustfmt::skip]
pub static ACTIONS: [Action; 60] = [
    action!(List, 1, "list", "1", Keys, "list keys", Always, Any),
    action!(VerifyKeys, 2, "list --verify", "V", Keys, "verify every key", Wallet, Any),
    action!(CreateFirst, 3, "create", "Enter", Keys, "create first wallet", FirstWallet, Any),
    action!(CreateKey, 4, "create", "n", Keys, "create another key", Wallet, Any),
    action!(CreateUnlock, 5, "create --unlock", "n", Keys, "create and unlock", Always, Any),
    action!(SetActive, 6, "change", "s", Keys, "make selected key active", Wallet, Any),
    action!(Unlock, 7, "unlock", "u", Session, "unlock for a duration", Wallet, Any),
    action!(PtraceFlag, 8, "unlock --allow-ptrace-scope-0", "start flag", Session, "accept ptrace scope 0 at startup", StartFlag, Any),
    action!(Lock, 9, "lock", "L", Session, "lock every wallet daemon", Always, Any),
    action!(Status, 10, "status", "7", Session, "read the session", Always, Any),
    action!(Backup, 11, "backup", "B", Keys, "recovery phrase in a shell", Always, Any),
    action!(Address, 12, "address", "4", Receive, "receive on this chain", OpenKey, Any),
    action!(Repair, 13, "address --verify", "A", Keys, "repair cached addresses", Wallet, Any),
    action!(Qr, 14, "qr", "Q", Receive, "QR code in a shell", OpenKey, Any),
    action!(Addresses, 15, "addresses", "4", Receive, "addresses on every family", OpenKey, Any),
    action!(AllBalances, 16, "balance", "a", Wallet, "balances on every network", OpenKey, Any),
    action!(Balance, 17, "balance --chain", "2", Wallet, "balances on this chain", OpenKey, Any),
    action!(TokenBalance, 18, "balance --token", "token", Tools, "read an arbitrary token balance", OpenKey, Evm),
    action!(Chains, 19, "chains", "c", Wallet, "choose a chain", Always, Any),
    action!(Rpc, 20, "--rpc", "e", Wallet, "choose a session RPC endpoint", Always, Any),
    action!(Transfer, 21, "transfer", "3/n", Send, "transfer to a checked payee", Signing, Any),
    action!(TokenTransfer, 22, "transfer --token", "Enter", Send, "transfer a registry token", Signing, Any),
    action!(DryRun, 23, "transfer --dry-run", "Enter", Send, "review a transfer without signing", Verified, Any),
    action!(Wait, 24, "transfer --wait-timeout", "w", Send, "wait for a send result", Pending, Any),
    action!(AllowPending, 25, "transfer --allow-pending", "p", Send, "review the pending-send override", Verified, Evm),
    action!(AllowDuplicate, 26, "transfer --allow-duplicate", "d", Send, "review the duplicate-send override", Verified, Any),
    action!(GasCap, 27, "transfer --max-fee-gwei", "g", Send, "set the review gas price ceiling", Verified, Evm),
    action!(SpeedUp, 28, "speed-up", "s", Send, "speed up a pending send", PendingSigning, Evm),
    action!(Cancel, 29, "cancel", "x", Send, "race a pending send with a cancel", PendingSigning, Evm),
    action!(Retry, 30, "retry", "R", Send, "retry this session's uncertain send", Retryable, Evm),
    action!(Tx, 31, "tx", "/", Wallet, "look up a transaction", Always, Any),
    action!(Call, 32, "call", "call", Tools, "read a contract", Always, Evm),
    action!(Invoke, 33, "invoke", "invoke", Tools, "review a listed contract call", Signing, Evm),
    action!(RawInvoke, 34, "invoke --data", "raw", Tools, "review listed raw calldata", Signing, Evm),
    action!(InvokeOptions, 35, "invoke", "p d g w s x b R", Tools, "contract-call send options", Signing, Evm),
    action!(SignMessage, 36, "sign-message", "sign", Tools, "review an ASCII message signature", Signing, Evm),
    action!(WhitelistList, 37, "whitelist list", "5", Whitelist, "read the whitelist", Wallet, Any),
    action!(WhitelistVerify, 38, "whitelist list --verify", "V", Whitelist, "verify the whitelist", Wallet, Any),
    action!(WhitelistAdd, 39, "whitelist add", "a", Whitelist, "add a payee", Wallet, Any),
    action!(WhitelistContract, 40, "whitelist add --contract", "a", Whitelist, "add a contract and selectors", Wallet, Evm),
    action!(WhitelistRemove, 41, "whitelist remove", "d", Whitelist, "remove a whitelist entry", Wallet, Any),
    action!(FeeCap, 42, "whitelist fee-cap", "f", Whitelist, "change a fee limit", Wallet, Any),
    action!(MessagePolicy, 43, "whitelist allow-message-signing", "m", Whitelist, "allow or refuse message signing", Wallet, Evm),
    action!(Reset, 44, "whitelist reset", "R", Whitelist, "reset the whitelist", Wallet, Any),
    action!(WhitelistUnlock, 45, "whitelist --unlock", "duration", Whitelist, "unlock after a whitelist change", Wallet, Any),
    action!(Startup, 46, "tui", "start flags", Session, "initial chain and warm-up duration", StartupInfo, Any),
    action!(SelectKey, 47, "--key", "Enter", Keys, "open a key here", Wallet, Any),
    action!(Version, 48, "--version", "?", Session, "version and help", Always, Any),
    action!(TronAddress, 49, "tron address", "4", Receive, "receive on Tron", OpenKey, Tron),
    action!(TronBalance, 50, "tron balance", "2", Wallet, "TRX and USDT balances", OpenKey, Tron),
    action!(TronTransfer, 51, "tron transfer", "3", Send, "send TRX or USDT", Signing, Tron),
    action!(TronTx, 52, "tron tx", "/", Wallet, "look up a Tron transaction", Always, Tron),
    action!(NativeCap, 60, "whitelist native-cap", "native-cap", Whitelist, "change the native amount limit", Wallet, Any),
    action!(TokenCap, 61, "whitelist token-cap", "token-cap", Whitelist, "change a token amount limit", Wallet, Any),
    action!(CalldataCap, 62, "whitelist calldata-cap", "calldata-cap", Whitelist, "change the calldata byte limit", Wallet, Any),
    action!(DuplicateWindow, 63, "whitelist duplicate-window", "duplicate-window", Whitelist, "change the advisory duplicate window", Wallet, Any),
    action!(TaskCreate, 64, "task create", "n", Tools, "create advisory task accounting", OpenKey, Any),
    action!(TaskList, 65, "task list", "a", Tools, "list local tasks", Wallet, Any),
    action!(TaskReport, 66, "task report", "report", Tools, "reconcile a task report", Wallet, Any),
    action!(TaskAttribution, 67, "--task", "t", Send, "attribute a reviewed transfer or invoke", Verified, Any),
];

pub fn by_id(id: ActionId) -> &'static Action {
    ACTIONS
        .iter()
        .find(|action| action.id == id)
        .expect("every ActionId has one table row")
}

/// Exact aliases win over substring matches, while duplicate aliases retain matrix order.
pub fn matching(query: &str) -> Vec<&'static Action> {
    let query = query.to_ascii_lowercase();
    let exact: Vec<_> = ACTIONS
        .iter()
        .filter(|action| action.alias == query)
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    ACTIONS
        .iter()
        .filter(|action| {
            action.alias.contains(&query) || action.label.to_ascii_lowercase().contains(&query)
        })
        .collect()
}

pub struct Menu {
    filter: Field,
    selected: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuResult {
    None,
    Close,
    Interrupt,
    Selected(ActionId),
    Disabled(&'static str),
}
impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}
impl Menu {
    pub fn new() -> Self {
        Self {
            filter: Field::new(Kind::Filter),
            selected: 0,
        }
    }
    pub fn filter(&self) -> &Field {
        &self.filter
    }
    pub fn selected_index(&self) -> usize {
        self.selected
    }
    pub fn rows(&self) -> Vec<&'static Action> {
        matching(self.filter.value())
    }
    pub fn title(&self) -> String {
        format!("actions ({})", ACTIONS.len())
    }
    pub fn update(&mut self, input: Input<'_>, context: &Context<'_>) -> MenuResult {
        match input {
            Input::Esc => return MenuResult::Close,
            Input::Interrupt => return MenuResult::Interrupt,
            Input::Up => self.selected = self.selected.saturating_sub(1),
            Input::Down => {
                self.selected = (self.selected + 1).min(self.rows().len().saturating_sub(1))
            }
            Input::Enter => {
                return match self.rows().get(self.selected) {
                    Some(action) => match action.availability(context) {
                        Availability::Ready => MenuResult::Selected(action.id),
                        Availability::Disabled(reason) => MenuResult::Disabled(reason),
                    },
                    None => MenuResult::Disabled("no matching action"),
                };
            }
            _ => {
                if self.filter.edit(input) == Edit::Changed {
                    self.selected = 0;
                }
            }
        }
        MenuResult::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_menu_has_every_coverage_alias_in_order_including_signed_limits() {
        let aliases = [
            "list",
            "list --verify",
            "create",
            "create",
            "create --unlock",
            "change",
            "unlock",
            "unlock --allow-ptrace-scope-0",
            "lock",
            "status",
            "backup",
            "address",
            "address --verify",
            "qr",
            "addresses",
            "balance",
            "balance --chain",
            "balance --token",
            "chains",
            "--rpc",
            "transfer",
            "transfer --token",
            "transfer --dry-run",
            "transfer --wait-timeout",
            "transfer --allow-pending",
            "transfer --allow-duplicate",
            "transfer --max-fee-gwei",
            "speed-up",
            "cancel",
            "retry",
            "tx",
            "call",
            "invoke",
            "invoke --data",
            "invoke",
            "sign-message",
            "whitelist list",
            "whitelist list --verify",
            "whitelist add",
            "whitelist add --contract",
            "whitelist remove",
            "whitelist fee-cap",
            "whitelist allow-message-signing",
            "whitelist reset",
            "whitelist --unlock",
            "tui",
            "--key",
            "--version",
            "tron address",
            "tron balance",
            "tron transfer",
            "tron tx",
            "whitelist native-cap",
            "whitelist token-cap",
            "whitelist calldata-cap",
            "whitelist duplicate-window",
            "task create",
            "task list",
            "task report",
            "--task",
        ];
        assert_eq!(ACTIONS.map(|a| a.alias), aliases);
        let rows: Vec<_> = ACTIONS.iter().map(|a| a.row).collect();
        assert_eq!(rows, (1..=52).chain(60..=67).collect::<Vec<_>>());
        let ids: std::collections::HashSet<_> = ACTIONS.iter().map(|a| a.id).collect();
        assert_eq!(ids.len(), ACTIONS.len());
        assert_eq!(Menu::new().title(), "actions (60)");
    }

    #[test]
    fn exact_filter_never_selects_a_more_dangerous_similarly_named_action() {
        let rows = matching("whitelist add");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, ActionId::WhitelistAdd);
        assert_eq!(rows[0].key, "a");
        assert_eq!(rows[0].tab, Tab::Whitelist);
        assert_eq!(
            matching("create").iter().map(|a| a.id).collect::<Vec<_>>(),
            [ActionId::CreateFirst, ActionId::CreateKey]
        );
        assert_eq!(
            matching("INVOKE").iter().map(|a| a.id).collect::<Vec<_>>(),
            [ActionId::Invoke, ActionId::InvokeOptions]
        );
        assert!(!matching("fee").is_empty());
    }

    #[test]
    fn enter_on_future_actions_or_missing_prerequisites_dispatches_nothing() {
        let mut menu = Menu::new();
        let mut context = Context::default();
        menu.update(Input::Paste("transfer"), &context);
        assert_eq!(
            menu.update(Input::Enter, &context),
            MenuResult::Disabled("in a later version")
        );
        context.implemented = &[ActionId::Transfer];
        assert_eq!(
            menu.update(Input::Enter, &context),
            MenuResult::Disabled("needs: a wallet")
        );
        context.has_wallet = true;
        context.has_open_key = true;
        assert_eq!(
            menu.update(Input::Enter, &context),
            MenuResult::Disabled("needs: V")
        );
        context.verified = true;
        assert_eq!(
            menu.update(Input::Enter, &context),
            MenuResult::Disabled("needs: a warm-up")
        );
        context.unlocked = true;
        assert_eq!(
            menu.update(Input::Enter, &context),
            MenuResult::Selected(ActionId::Transfer)
        );
        assert!(
            matches!(by_id(ActionId::PtraceFlag).availability(&context), Availability::Disabled(reason) if reason.starts_with("start flag:"))
        );
    }

    #[test]
    fn menu_characters_are_filter_input_and_control_pastes_change_nothing() {
        let mut menu = Menu::new();
        for c in "q?1jk".chars() {
            assert_eq!(
                menu.update(Input::Char(c), &Context::default()),
                MenuResult::None
            );
        }
        assert_eq!(menu.filter().value(), "q?1jk");
        menu.update(Input::Paste("\u{1b}[2J█"), &Context::default());
        assert_eq!(menu.filter().value(), "q?1jk");
        assert_eq!(
            menu.update(Input::Enter, &Context::default()),
            MenuResult::Disabled("no matching action")
        );
        assert_eq!(
            menu.update(Input::Esc, &Context::default()),
            MenuResult::Close
        );
    }

    #[test]
    fn waiting_for_an_existing_send_does_not_require_signing_to_stay_unlocked() {
        let context = Context {
            implemented: &[ActionId::Wait],
            pending_send: true,
            ..Context::default()
        };
        assert_eq!(
            by_id(ActionId::Wait).availability(&context),
            Availability::Ready
        );
    }

    #[test]
    fn tab_navigation_covers_exactly_seven_tabs_and_wraps() {
        for (index, tab) in Tab::ALL.iter().enumerate() {
            assert_eq!(Tab::from_number(index as u8 + 1), Some(*tab));
            assert_eq!(tab.next().previous(), *tab);
        }
        assert_eq!(Tab::Keys.previous(), Tab::Session);
        assert_eq!(Tab::Session.next(), Tab::Keys);
        assert_eq!(Tab::from_number(0), None);
        assert_eq!(Tab::from_number(8), None);
    }
}

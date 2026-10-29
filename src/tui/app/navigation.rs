//! Read-only navigation, fields and pane data. It shares App's generations and password pins.

use super::*;
use crate::chains::{self, CHAINS, PRIMARY_CHAIN};
use crate::core::amount::format_units;
use crate::core::time::parse_duration;
use crate::tui::actions::{ActionId, Context, Menu, MenuResult, Tab};
use crate::tui::form::{Edit, Field, Input, Kind};
use crate::wallet;
use std::fmt;
use zeroize::Zeroizing;

pub struct UserText(pub Zeroizing<String>);
impl fmt::Debug for UserText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserText")
            .field("length", &self.0.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RpcChoice {
    pub chain: &'static str,
    pub url: String,
}
impl fmt::Debug for RpcChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcChoice")
            .field("chain", &self.chain)
            .field("url", &chains::redact_url(&self.url))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldPurpose {
    Duration,
    TxHash,
    Rpc(&'static str),
}
pub enum Popup {
    Menu(Menu),
    Chains {
        selected: usize,
    },
    Field {
        purpose: FieldPurpose,
        field: Field,
    },
    CheckingRpc,
    Error(Outside),
    Lines {
        title: &'static str,
        lines: Vec<Outside>,
    },
}

pub struct Navigation {
    pub tab: Tab,
    pub popup: Option<Popup>,
    pub popup_notice: Option<Outside>,
    pub lines: Pane<Vec<Outside>>,
    pub scroll: usize,
    pub popup_scroll: usize,
    pub every: bool,
    pub session: Option<StatusView>,
    pub home: Outside,
    pub allow_ptrace_scope_0: bool,
    nets: BTreeMap<&'static str, Net>,
    pub(super) family_reads: BTreeMap<(String, &'static str), (String, Trust)>,
    pub(super) family_pins: BTreeMap<(String, &'static str), String>,
    /// The single-network balance lines that are an asset: line index -> its token contract
    /// (`None` the native coin). Enter on such a row starts a payment of that asset.
    pub(super) balance_assets: BTreeMap<usize, Option<String>>,
    rpc_request: u64,
    pending_rpc: Option<u64>,
}
impl Navigation {
    pub fn new(first: &Net) -> Self {
        let mut nets: BTreeMap<_, _> = CHAINS
            .iter()
            .map(|chain| (chain.key, wallet::net(chain)))
            .collect();
        nets.insert(first.chain.key, first.clone());
        Self {
            tab: Tab::Keys,
            popup: None,
            popup_notice: None,
            lines: Pane::Loading,
            scroll: 0,
            popup_scroll: 0,
            every: false,
            session: None,
            home: Outside::home(""),
            allow_ptrace_scope_0: false,
            nets,
            family_pins: BTreeMap::new(),
            family_reads: BTreeMap::new(),
            balance_assets: BTreeMap::new(),
            rpc_request: 0,
            pending_rpc: None,
        }
    }
}

const IMPLEMENTED: &[ActionId] = &[
    ActionId::WhitelistList,
    ActionId::WhitelistVerify,
    ActionId::WhitelistAdd,
    ActionId::WhitelistContract,
    ActionId::WhitelistRemove,
    ActionId::FeeCap,
    ActionId::NativeCap,
    ActionId::TokenCap,
    ActionId::CalldataCap,
    ActionId::MessagePolicy,
    ActionId::Reset,
    ActionId::WhitelistUnlock,
    ActionId::CreateFirst,
    ActionId::CreateKey,
    ActionId::CreateUnlock,
    ActionId::Backup,
    ActionId::List,
    ActionId::VerifyKeys,
    ActionId::SetActive,
    ActionId::Unlock,
    ActionId::PtraceFlag,
    ActionId::Lock,
    ActionId::Status,
    ActionId::Address,
    ActionId::Repair,
    ActionId::Qr,
    ActionId::Addresses,
    ActionId::AllBalances,
    ActionId::Balance,
    ActionId::Chains,
    ActionId::Rpc,
    ActionId::Startup,
    ActionId::SelectKey,
    ActionId::Version,
    ActionId::Call,
    ActionId::Invoke,
    ActionId::RawInvoke,
    ActionId::SignMessage,
    ActionId::TokenBalance,
    ActionId::InvokeOptions,
    ActionId::Transfer,
    ActionId::TokenTransfer,
    ActionId::DryRun,
    ActionId::Wait,
    ActionId::AllowPending,
    ActionId::AllowDuplicate,
    ActionId::GasCap,
    ActionId::SpeedUp,
    ActionId::Cancel,
    ActionId::Retry,
    ActionId::Tx,
    ActionId::TronAddress,
    ActionId::TronBalance,
    ActionId::TronTransfer,
    ActionId::TronTx,
];

impl App {
    pub fn navigation(&self) -> &Navigation {
        &self.nav
    }
    pub fn tab(&self) -> Tab {
        self.nav.tab
    }
    pub fn net(&self) -> &Net {
        &self.nav.nets[self.chain.key]
    }
    pub fn net_for(&self, key: &str) -> Option<&Net> {
        self.nav.nets.get(key)
    }
    pub fn nets(&self) -> Vec<Net> {
        self.nav.nets.values().cloned().collect()
    }
    pub fn configure(&mut self, home: &std::path::Path, allow_ptrace_scope_0: bool) {
        self.whitelist.home = home.to_path_buf();
        self.nav.home = Outside::home(&home.to_string_lossy());
        self.nav.allow_ptrace_scope_0 = allow_ptrace_scope_0;
    }
    pub fn menu_context(&self) -> Context<'static> {
        let (pending_send, retryable_send) = self.replacement_available();
        Context {
            implemented: IMPLEMENTED,
            has_wallet: matches!(&self.status, Pane::Ready(bar) if bar.configured),
            has_open_key: self.open_key.is_some(),
            verified: self
                .open_key
                .as_ref()
                .is_some_and(|key| self.family_pinned(key, self.chain)),
            unlocked: matches!(&self.status, Pane::Ready(bar) if matches!(bar.lock, LockLine::Open(_))),
            is_tron: self.chain.key == "tron",
            pending_send,
            retryable_send,
        }
    }
    pub fn family_pinned(&self, key: &str, chain: &Chain) -> bool {
        self.family_pin(key, chain).is_some()
    }
    pub(super) fn family_pin(&self, key: &str, chain: &Chain) -> Option<&String> {
        self.nav
            .family_pins
            .get(&(key.to_string(), chain.family.name()))
            .or_else(|| {
                (chain.family.name()
                    == chains::find(PRIMARY_CHAIN)
                        .expect("primary chain")
                        .family
                        .name())
                .then(|| self.pins.get(key))
                .flatten()
            })
    }
    pub(super) fn check_family_pin(&self, key: &str, chain: &Chain, address: &str) -> Result<()> {
        super::addresses::validate_family_address(chain, address)?;
        if let Some(pinned) = self.family_pin(key, chain) {
            let matches = if chain.family.name() == "evm" {
                pinned.eq_ignore_ascii_case(address)
            } else {
                pinned == address
            };
            if !matches {
                return Err(WalletError::validation(format!(
                    "the {} address for key '{}' ({}) does not match the address this TUI derived from the seed ({}). Do not send funds to either address. Run zwallet address --verify",
                    chain.key,
                    crate::core::text::ascii_inert(key, 32),
                    crate::core::text::ascii_inert(address, 42),
                    pinned
                )));
            }
        }
        Ok(())
    }

    pub(super) fn navigation_update(
        &mut self,
        msg: Msg,
    ) -> std::ops::ControlFlow<Msg, Vec<Effect>> {
        let work = match msg {
            Msg::Paste(text) => {
                if self.password != PasswordState::Idle {
                    Vec::new()
                } else {
                    self.popup_input(Input::Paste(&text.0)).unwrap_or_default()
                }
            }
            Msg::Verified(result) => {
                let result = result.and_then(|(checked, list)| {
                    // Validate the whole result before inserting even one new lifetime pin.
                    for row in &checked.opened.keys {
                        if self.verified_indices.get(&row.name).is_some_and(|index| *index != row.index) {
                            return Err(WalletError::validation("the key index changed after the password check; close and verify this wallet again"));
                        }
                        if let Some(address) = &row.address {
                            self.check_pin(&row.name, address)?;
                        }
                    }
                    let mut derived = BTreeMap::new();
                    for row in &checked.addresses {
                        let chain = chains::find(row.chain)?;
                        if row.family != chain.family.name()
                            || !checked.opened.keys.iter().any(|key| key.name == row.key)
                        {
                            return Err(WalletError::validation(
                                "password-derived address family is inconsistent",
                            ));
                        }
                        if let Some(previous) = derived.insert((&row.key, row.family), &row.address)
                            && previous != &row.address
                        {
                            return Err(WalletError::validation(
                                "password-derived family addresses disagree",
                            ));
                        }
                        self.check_family_pin(&row.key, chain, &row.address)?;
                    }
                    for row in &checked.opened.keys {
                        if !checked.addresses.iter().any(|a| {
                            a.key == row.name
                                && a.chain == PRIMARY_CHAIN
                                && Some(&a.address) == row.address.as_ref()
                        }) {
                            return Err(WalletError::validation(
                                "password-derived address set is incomplete",
                            ));
                        }
                    }
                    Ok((checked, list))
                });
                match result {
                    Ok((checked, list)) => {
                        for row in checked.addresses {
                            self.nav
                                .family_pins
                                .insert((row.key, row.family), row.address);
                        }
                        if self.password_purpose == PasswordPurpose::Repair {
                            let mut lines = Vec::new();
                            for repair in checked.repairs {
                                lines.push(Outside::detail(&format!(
                                    "key {} / {}",
                                    repair.key, repair.chain
                                )));
                                lines.push(Outside::detail(&format!(
                                    "was (do not fund): {}",
                                    crate::core::text::ascii_inert(&repair.was, 48)
                                )));
                                lines.push(Outside::detail(&format!("now: {}", repair.now)));
                            }
                            if lines.is_empty() {
                                lines.push(Outside::detail("No cached address needed repair."));
                            }
                            self.nav.popup = Some(Popup::Lines {
                                title: "address repair",
                                lines,
                            });
                        }
                        self.update(Msg::Checked(Ok((checked.opened, list))))
                    }
                    Err(error) => self.update(Msg::Checked(Err(error))),
                }
            }
            Msg::Balances(generation, key, result) => {
                if generation != self.generation
                    || self.open_key.as_ref() != Some(&key)
                    || self.nav.tab != Tab::Wallet
                    || self.nav.every
                {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                match result {
                    Ok(read) => {
                        if read.chain.key != self.chain.key || read.key != key {
                            self.fail_pane(WalletError::validation(
                                "balance reply belongs to another chain or key",
                            ));
                            return std::ops::ControlFlow::Continue(Vec::new());
                        }
                        if let Err(error) = self.observe_family_address(
                            &key,
                            read.chain,
                            &read.address,
                            read.trust.clone(),
                        ) {
                            self.fail_pane(error);
                            return std::ops::ControlFlow::Continue(Vec::new());
                        }
                        let mut lines =
                            vec![Outside::detail(&format!("{} balances", read.chain.name))];
                        self.nav.balance_assets.clear();
                        if !read.native.raw.is_zero() {
                            self.nav.balance_assets.insert(lines.len(), None);
                            lines.push(amount_line(&read.native));
                        }
                        for token in read.tokens {
                            match token.raw {
                                Ok(raw) if !raw.is_zero() => {
                                    self.nav
                                        .balance_assets
                                        .insert(lines.len(), Some(token.address.clone()));
                                    lines.push(amount_line(&chains::Balance {
                                        symbol: token.symbol,
                                        decimals: token.decimals,
                                        raw,
                                    }))
                                }
                                Err(error) => lines.push(Outside::detail(&format!(
                                    "{} error: {}",
                                    token.symbol,
                                    wallet::brief(&error.message)
                                ))),
                                _ => {}
                            }
                        }
                        if read.activated == Some(false) {
                            lines.push(Outside::detail("not activated"));
                        }
                        if let Some(warning) = read.warning {
                            lines.push(Outside::detail(if warning.contains("do not fund") {
                                "Account warning: do not fund; Tab then j/k for details"
                            } else {
                                "Account warning: Tab then j/k for details"
                            }));
                            let warning = Outside::warning(&warning);
                            let chars: Vec<_> = warning.text().chars().collect();
                            lines.extend(
                                chars.chunks(76).take(7).map(|chunk| {
                                    Outside::warning(&chunk.iter().collect::<String>())
                                }),
                            );
                        }
                        if lines.len() == 1 {
                            lines.push(Outside::detail("No non-zero balance on this network."));
                        }
                        self.nav.lines = Pane::Ready(lines);
                    }
                    Err(error) => self.read_failed(error),
                }
                Vec::new()
            }
            Msg::EveryBalances(generation, key, result) => {
                if generation != self.generation
                    || self.open_key.as_ref() != Some(&key)
                    || self.nav.tab != Tab::Wallet
                    || !self.nav.every
                {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                match result {
                    Ok(read) => {
                        for network in &read.networks {
                            if let Some(address) = &network.address
                                && let Err(error) =
                                    self.check_family_pin(&key, network.chain, address)
                            {
                                self.fail_pane(error);
                                return std::ops::ControlFlow::Continue(Vec::new());
                            }
                        }
                        let total = read.networks.len();
                        let (mut held, mut failed) = (0, 0);
                        let mut lines = Vec::new();
                        for network in read.networks {
                            let text = match network.balances {
                                Ok(wallet::read::NetworkRead {
                                    native,
                                    tokens,
                                    activated,
                                    warning,
                                }) => {
                                    let mut amounts = vec![format!(
                                        "{} {}",
                                        format_units(native.raw, native.decimals),
                                        native.symbol
                                    )];
                                    if activated == Some(false) {
                                        amounts.push("not activated".into());
                                    }
                                    if let Some(warning) = warning {
                                        amounts.push(format!("warning: {warning}"));
                                    }
                                    let mut any = !native.raw.is_zero();
                                    for token in tokens {
                                        match token.raw {
                                            Ok(raw) if !raw.is_zero() => {
                                                any = true;
                                                amounts.push(format!(
                                                    "{} {}",
                                                    format_units(raw, token.decimals),
                                                    token.symbol
                                                ));
                                            }
                                            Err(_) => {
                                                amounts.push(format!("{} error", token.symbol))
                                            }
                                            _ => {}
                                        }
                                    }
                                    if any {
                                        held += 1;
                                    }
                                    format!("{}: {}", network.chain.key, amounts.join(", "))
                                }
                                Err(error) => {
                                    failed += 1;
                                    format!(
                                        "{}: error: {}",
                                        network.chain.key,
                                        wallet::brief(&error.message)
                                    )
                                }
                            };
                            lines.push(Outside::detail(&text));
                        }
                        lines.push(Outside::detail(&format!(
                            "{held} of {total} networks have a balance; {failed} did not answer"
                        )));
                        self.nav.lines = Pane::Ready(lines);
                    }
                    Err(error) => self.read_failed(error),
                }
                Vec::new()
            }
            Msg::Addresses(generation, key, result) => {
                if generation != self.generation
                    || self.open_key.as_ref() != Some(&key)
                    || self.nav.tab != Tab::Receive
                {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                match result {
                    Ok(read) => {
                        for row in &read.rows {
                            if row.error.is_some() {
                                continue;
                            }
                            if let Err(error) = self.check_family_pin(&key, row.chain, &row.address)
                            {
                                self.fail_pane(error);
                                return std::ops::ControlFlow::Continue(Vec::new());
                            }
                        }
                        let mut lines = Vec::new();
                        if let Some(row) = read.rows.iter().find(|r| r.chain.key == self.chain.key)
                        {
                            if let Err(error) = self.observe_family_address(
                                &key,
                                row.chain,
                                &row.address,
                                read.trust.clone(),
                            ) {
                                self.fail_pane(error);
                                return std::ops::ControlFlow::Continue(Vec::new());
                            }
                            lines.push(Outside::detail(&format!("{} / {}", self.chain.name, key)));
                            lines.push(Outside::address(&row.address));
                            let trust = if matches!(read.trust, Trust::Unverified(Some(_))) {
                                "not verified: another process answered"
                            } else if self.family_pinned(&key, self.chain) {
                                "verified by password"
                            } else if read.trust.verified() {
                                "verified by session"
                            } else {
                                "not verified: V checks from the seed before funding"
                            };
                            lines.push(Outside::detail(&format!("Address trust: {trust}")));
                        }
                        lines.push(Outside::detail("Addresses by network:"));
                        for row in read.rows {
                            let value = match row.error {
                                Some(error) => format!("error: {}", wallet::brief(&error.message)),
                                None => row.address,
                            };
                            lines.push(Outside::detail(&format!("{}: {}", row.chain.key, value)));
                        }
                        if let Some(warning) = read.trust.warning() {
                            lines.push(Outside::detail(&format!("warning: {warning}")));
                        }
                        self.nav.lines = Pane::Ready(lines);
                    }
                    Err(error) => self.read_failed(error),
                }
                Vec::new()
            }
            Msg::RpcChecked(request, result) => {
                if self.nav.pending_rpc != Some(request) {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                self.nav.pending_rpc = None;
                match result {
                    Ok(choice) => {
                        let chain = chains::find(choice.chain).expect("chosen registered chain");
                        let net = wallet::net_with(chain, &choice.url);
                        self.rpc_host = Outside::rpc_host(&net.rpc_url);
                        self.nav.nets.insert(chain.key, net);
                        self.chain = chain;
                        self.nav.popup = None;
                        self.reload()
                    }
                    Err(error) => {
                        self.nav.popup = Some(Popup::Error(Outside::error(&error)));
                        Vec::new()
                    }
                }
            }
            other => return std::ops::ControlFlow::Break(other),
        };
        std::ops::ControlFlow::Continue(work)
    }

    pub(super) fn load_view(&mut self) -> Vec<Effect> {
        self.nav.scroll = 0;
        if self.nav.tab == Tab::Keys {
            return Vec::new();
        }
        if self.nav.tab == Tab::Session {
            self.render_session();
            return self.load_status();
        }
        if self.nav.tab == Tab::Send {
            return self.show_transfer();
        }
        if self.nav.tab == Tab::Whitelist {
            return self.load_whitelist();
        }
        if self.nav.tab == Tab::Tools {
            return self.show_tools();
        }
        let Some(key) = self.open_key.clone() else {
            self.nav.lines = Pane::Ready(vec![Outside::detail("Open a key with Enter on 1 keys.")]);
            return Vec::new();
        };
        self.nav.lines = Pane::Loading;
        let mut work = vec![match self.nav.tab {
            Tab::Wallet if self.nav.every => Effect::LoadEveryBalances(self.generation, key),
            Tab::Wallet => Effect::LoadBalances(self.generation, key),
            Tab::Receive => Effect::LoadAddresses(self.generation, key),
            _ => return Vec::new(),
        }];
        work.extend(self.load_history_owner());
        work
    }

    pub(super) fn navigation_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        if let Some(work) = self.popup_input(input(key)) {
            return Some(work);
        }
        if let Some(work) = self.history_key(key) {
            return Some(work);
        }
        if let Some(work) = self.whitelist_input(input(key)) {
            return Some(work);
        }
        if let Key::Char(c @ '1'..='7') = key {
            self.nav.tab = Tab::from_number(c as u8 - b'0').expect("tab key");
            return Some(self.reload());
        }
        match key {
            Key::Char('[') | Key::Char(']') => {
                self.nav.tab = if key == Key::Char('[') {
                    self.nav.tab.previous()
                } else {
                    self.nav.tab.next()
                };
                Some(self.reload())
            }
            Key::Char('?') | Key::Char(':') => {
                self.nav.popup_scroll = 0;
                self.nav.popup = Some(Popup::Menu(Menu::new()));
                Some(Vec::new())
            }
            Key::Char('/') if self.nav.tab == Tab::Wallet => {
                if self.nav.every {
                    return Some(self.run_action(ActionId::Tx));
                }
                self.nav.popup = Some(Popup::Field {
                    purpose: FieldPurpose::TxHash,
                    field: Field::new(
                        if matches!(self.chain.family, crate::chains::Family::Tron(_)) {
                            Kind::TronTxHash
                        } else {
                            Kind::TxHash
                        },
                    ),
                });
                Some(Vec::new())
            }
            Key::Char('c') => {
                self.chain_picker();
                Some(Vec::new())
            }
            Key::Char('V') => Some(self.begin_password(PasswordPurpose::Verify {
                all: self.nav.tab == Tab::Keys,
            })),
            Key::Char('A') if self.nav.tab == Tab::Keys => {
                Some(self.begin_password(PasswordPurpose::Repair))
            }
            Key::Char('u') => {
                self.duration_field();
                Some(Vec::new())
            }
            Key::Char('a') if self.nav.tab == Tab::Wallet => {
                self.nav.every = true;
                self.invalidate_reads();
                Some(self.load_view())
            }
            Key::Char('Q') if self.nav.tab == Tab::Receive => {
                self.nav.popup = Some(Popup::Lines {
                    title: "QR code",
                    lines: vec![Outside::detail("zwallet address shows it as a QR code")],
                });
                Some(Vec::new())
            }
            Key::Up | Key::Char('k') if self.nav.tab != Tab::Keys => {
                self.nav.scroll = self.nav.scroll.saturating_sub(1);
                Some(Vec::new())
            }
            Key::Down | Key::Char('j') if self.nav.tab != Tab::Keys => {
                let count = match &self.nav.lines {
                    Pane::Ready(lines) => lines.len(),
                    _ => 0,
                };
                self.nav.scroll = (self.nav.scroll + 1).min(count.saturating_sub(1));
                Some(Vec::new())
            }
            Key::Esc => {
                self.nav.every = false;
                Some(self.load_view())
            }
            Key::Enter if self.nav.tab == Tab::Wallet => {
                // With the balances focused, the top row drawn is the chosen one; an asset row
                // starts a payment of that asset.
                let asset = (self.history.balance_focus && !self.nav.every)
                    .then(|| self.nav.balance_assets.get(&self.nav.scroll).cloned())
                    .flatten();
                self.nav.tab = Tab::Send;
                Some(match asset {
                    Some(token) => self.start_payment(token),
                    None => self.show_transfer(),
                })
            }
            Key::Enter | Key::Char('s') if self.nav.tab != Tab::Keys => Some(Vec::new()),
            _ => None,
        }
    }

    pub(super) fn begin_password(&mut self, purpose: PasswordPurpose) -> Vec<Effect> {
        if self.locking || self.activating {
            return Vec::new();
        }
        let key = self
            .open_key
            .clone()
            .or_else(|| match &self.keys {
                Pane::Ready(list) => list.rows.get(self.selected).map(|row| row.key.clone()),
                _ => None,
            })
            .unwrap_or_default();
        self.pending_key = Some(key);
        self.password_purpose = purpose;
        self.password = PasswordState::Preparing;
        self.nav.popup = None;
        self.invalidate_reads();
        vec![match purpose {
            PasswordPurpose::Warm => Effect::PrepareUnlock,
            PasswordPurpose::Unlock(ttl) => Effect::PrepareDuration(ttl),
            PasswordPurpose::Verify { .. }
            | PasswordPurpose::Repair
            | PasswordPurpose::WhitelistVerify => Effect::PrepareRead,
            PasswordPurpose::WhitelistEdit => unreachable!("whitelist owns its plan"),
        }]
    }
    fn chain_picker(&mut self) {
        let selected = CHAINS
            .iter()
            .position(|chain| chain.key == self.chain.key)
            .unwrap_or(0);
        self.nav.popup = Some(Popup::Chains { selected });
    }
    fn duration_field(&mut self) {
        self.nav.popup = Some(Popup::Field {
            purpose: FieldPurpose::Duration,
            field: Field::new(Kind::Duration),
        });
    }

    fn popup_input(&mut self, input: Input<'_>) -> Option<Vec<Effect>> {
        let popup = self.nav.popup.take()?;
        self.nav.popup_notice = None;
        match popup {
            Popup::Menu(mut menu) => {
                match menu.update(input, &self.menu_context()) {
                    MenuResult::Selected(action) => return Some(self.run_action(action)),
                    MenuResult::Close => return Some(Vec::new()),
                    MenuResult::Interrupt => {
                        self.quit = true;
                        return Some(Vec::new());
                    }
                    MenuResult::Disabled(_) => {}
                    MenuResult::None => {}
                }
                self.nav.popup = Some(Popup::Menu(menu));
            }
            Popup::Chains { mut selected } => {
                match input {
                    Input::Up | Input::Char('k') => selected = selected.saturating_sub(1),
                    Input::Down | Input::Char('j') => {
                        selected = (selected + 1).min(CHAINS.len() - 1)
                    }
                    Input::Esc => return Some(Vec::new()),
                    // A list, not a text field: the popup closes and the tab keys act.
                    tab if switches_tab(&tab) => return None,
                    Input::Interrupt => {
                        self.quit = true;
                        return Some(Vec::new());
                    }
                    Input::Char('e') => {
                        self.nav.popup = Some(Popup::Field {
                            purpose: FieldPurpose::Rpc(CHAINS[selected].key),
                            field: Field::new(Kind::RpcUrl),
                        });
                        return Some(Vec::new());
                    }
                    Input::Enter => {
                        self.chain = &CHAINS[selected];
                        self.rpc_host = Outside::rpc_host(&self.net().rpc_url);
                        return Some(self.reload());
                    }
                    _ => {}
                }
                self.nav.popup = Some(Popup::Chains { selected });
            }
            Popup::Field { purpose, mut field } => {
                match field.edit(input) {
                    Edit::Submit => match purpose {
                        FieldPurpose::TxHash => {
                            if self.history.scope.is_some() {
                                return Some(self.open_transaction(field.value()));
                            }
                            self.nav.popup_notice = Some(Outside::detail(
                                "wait for the selected key's address, then press Enter",
                            ));
                        }
                        FieldPurpose::Duration => match parse_duration(field.value()) {
                            Ok(ttl) => {
                                return Some(self.begin_password(PasswordPurpose::Unlock(ttl)));
                            }
                            Err(error) => self.nav.popup_notice = Some(Outside::error(&error)),
                        },
                        FieldPurpose::Rpc(chain) => {
                            self.nav.rpc_request += 1;
                            self.nav.pending_rpc = Some(self.nav.rpc_request);
                            self.nav.popup = Some(Popup::CheckingRpc);
                            return Some(vec![Effect::CheckRpc(
                                self.nav.rpc_request,
                                RpcChoice {
                                    chain,
                                    url: field.value().to_string(),
                                },
                            )]);
                        }
                    },
                    Edit::Cancel => return Some(Vec::new()),
                    Edit::Interrupt => {
                        self.quit = true;
                        return Some(Vec::new());
                    }
                    Edit::Rejected(reason) => self.nav.popup_notice = Some(Outside::detail(reason)),
                    _ => {}
                }
                self.nav.popup = Some(Popup::Field { purpose, field });
            }
            Popup::Lines { title, lines } => match input {
                Input::Esc | Input::Enter => {
                    self.nav.popup_scroll = 0;
                }
                tab if switches_tab(&tab) => {
                    self.nav.popup_scroll = 0;
                    return None;
                }
                Input::Interrupt => self.quit = true,
                Input::Up | Input::Char('k') => {
                    self.nav.popup_scroll = self.nav.popup_scroll.saturating_sub(1);
                    self.nav.popup = Some(Popup::Lines { title, lines });
                }
                Input::Down | Input::Char('j') => {
                    self.nav.popup_scroll =
                        (self.nav.popup_scroll + 1).min(lines.len().saturating_sub(1));
                    self.nav.popup = Some(Popup::Lines { title, lines });
                }
                _ => self.nav.popup = Some(Popup::Lines { title, lines }),
            },
            other => match input {
                Input::Esc | Input::Enter => {
                    self.nav.pending_rpc = None;
                }
                Input::Interrupt => {
                    self.nav.pending_rpc = None;
                    self.quit = true;
                }
                _ => self.nav.popup = Some(other),
            },
        }
        Some(Vec::new())
    }

    fn run_action(&mut self, action: ActionId) -> Vec<Effect> {
        let tron_alias = match action {
            ActionId::TronAddress => Some(ActionId::Address),
            ActionId::TronBalance => Some(ActionId::Balance),
            ActionId::TronTransfer => Some(ActionId::Transfer),
            ActionId::TronTx => Some(ActionId::Tx),
            _ => None,
        };
        let action = if let Some(action) = tron_alias {
            // The Tron coverage rows use the explicit chain picker and ordinary tabs.
            // An alias never changes the endpoint or the key behind the human's review.
            if !self.on_tron() {
                self.nav.popup_notice = Some(Outside::detail("select Tron with c"));
                return Vec::new();
            }
            action
        } else {
            action
        };
        if let Some(work) = self.whitelist_action(action) {
            return work;
        }
        if let Some(work) = self.replacement_action(action) {
            return work;
        }
        if let Some(work) = self.transfer_action(action) {
            return work;
        }
        match action {
            ActionId::CreateFirst | ActionId::CreateKey | ActionId::CreateUnlock => {
                return self.begin_create();
            }
            ActionId::Backup => return self.backup_hint(),
            ActionId::Call => return self.start_tool(super::tools::Tool::Call),
            ActionId::Invoke | ActionId::InvokeOptions => {
                return self.start_tool(super::tools::Tool::Invoke);
            }
            ActionId::RawInvoke => return self.start_tool(super::tools::Tool::RawInvoke),
            ActionId::SignMessage => return self.start_tool(super::tools::Tool::Message),
            ActionId::TokenBalance => return self.start_tool(super::tools::Tool::Token),
            ActionId::List => self.nav.tab = Tab::Keys,
            ActionId::VerifyKeys => {
                return self.begin_password(PasswordPurpose::Verify { all: true });
            }
            ActionId::SetActive => {
                self.nav.tab = Tab::Keys;
                return self.key(Key::Char('s'));
            }
            ActionId::SelectKey => {
                self.nav.tab = Tab::Keys;
                return self.key(Key::Enter);
            }
            ActionId::Unlock => {
                self.duration_field();
                return Vec::new();
            }
            ActionId::Lock => return self.key(Key::Char('L')),
            ActionId::Status => self.nav.tab = Tab::Session,
            ActionId::Address | ActionId::Addresses => self.nav.tab = Tab::Receive,
            ActionId::Repair => return self.begin_password(PasswordPurpose::Repair),
            ActionId::Qr => {
                self.nav.tab = Tab::Receive;
                return self.navigation_key(Key::Char('Q')).unwrap_or_default();
            }
            ActionId::Balance | ActionId::AllBalances => {
                self.nav.tab = Tab::Wallet;
                self.nav.every = action == ActionId::AllBalances;
            }
            ActionId::Chains | ActionId::Rpc => {
                self.chain_picker();
                return Vec::new();
            }
            ActionId::Tx => {
                self.nav.tab = Tab::Wallet;
                self.nav.every = false;
                let work = self.reload();
                self.nav.popup = Some(Popup::Field {
                    purpose: FieldPurpose::TxHash,
                    field: Field::new(
                        if matches!(self.chain.family, crate::chains::Family::Tron(_)) {
                            Kind::TronTxHash
                        } else {
                            Kind::TxHash
                        },
                    ),
                });
                return work;
            }
            ActionId::Version => {
                self.nav.popup = Some(Popup::Lines {
                    title: "version",
                    lines: vec![Outside::detail(&crate::version_line())],
                });
                return Vec::new();
            }
            _ => {
                self.nav.popup_notice = Some(Outside::detail("in a later version"));
                return Vec::new();
            }
        }
        self.reload()
    }

    fn read_failed(&mut self, error: WalletError) {
        use crate::core::error::ErrorCode;
        if matches!(
            error.code,
            ErrorCode::ValidationError | ErrorCode::InvalidAddress
        ) {
            self.fail_pane(error);
        } else {
            self.nav.lines = Pane::Failed(Outside::error(&error));
        }
    }

    pub(super) fn render_session(&mut self) {
        let mut lines = vec![Outside::detail("Wallet home:"), self.nav.home.clone()];
        if self.nav.allow_ptrace_scope_0 {
            lines.push(Outside::detail("ptrace_scope 0 accepted"));
        }
        if let Some(status) = &self.nav.session {
            lines.push(Outside::detail(if status.locked {
                "This home: locked"
            } else {
                "This home: signing is unlocked"
            }));
            let own: Vec<_> = status.daemon_pids.iter().map(i32::to_string).collect();
            lines.push(Outside::detail(&format!(
                "This home's daemon pids: {}",
                own.join(", ")
            )));
            if let Some(serving) = &status.serving {
                if let Some(until) = &serving.expires_at {
                    lines.push(Outside::detail(&format!("Expires: {until}")));
                }
                let generation = serving
                    .whitelist
                    .get("gen")
                    .and_then(serde_json::Value::as_u64);
                let count = serving.whitelist.get("entries").and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_array().map(|a| a.len() as u64))
                });
                lines.push(Outside::detail(&format!(
                    "reports gen {}, {} entries (not checked here)",
                    generation
                        .map(|g| g.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    count
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "unknown".into())
                )));
            }
            lines.push(Outside::detail(
                "Other homes' daemon pids (L stops these too):",
            ));
            for pid in status.other_daemon_pids.iter().take(8) {
                lines.push(Outside::detail(&pid.to_string()));
            }
            if status.other_daemon_pids.len() > 8 {
                lines.push(Outside::detail(&format!(
                    "{} more",
                    status.other_daemon_pids.len() - 8
                )));
            }
            if status.other_daemon_pids.is_empty() {
                lines.push(Outside::detail("none"));
            }
        } else {
            lines.push(Outside::detail("reading session..."));
        }
        self.nav.lines = Pane::Ready(lines);
    }
}

/// The global tab keys, `1`–`7`, `[` and `]`. Only a focused text field keeps them
/// (.prd/03-tui.md §Screens and keys); a list, a review or an outcome lets them through.
pub(super) fn switches_tab(input: &Input<'_>) -> bool {
    matches!(input, Input::Char('1'..='7' | '[' | ']'))
}

pub(super) fn input(key: Key) -> Input<'static> {
    match key {
        Key::Char(c) => Input::Char(c),
        Key::Up => Input::Up,
        Key::Down => Input::Down,
        Key::Left => Input::Left,
        Key::Right => Input::Right,
        Key::Home => Input::Home,
        Key::End => Input::End,
        Key::Backspace => Input::Backspace,
        Key::Delete => Input::Delete,
        Key::Tab => Input::Tab,
        Key::BackTab => Input::BackTab,
        Key::Enter => Input::Enter,
        Key::Esc => Input::Esc,
        Key::Interrupt => Input::Interrupt,
    }
}
fn amount_line(balance: &chains::Balance) -> Outside {
    Outside::detail(&format!(
        "{} {}",
        format_units(balance.raw, balance.decimals),
        balance.symbol
    ))
}

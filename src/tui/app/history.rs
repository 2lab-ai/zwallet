//! History reads carry their selected key, chain, endpoint and owner through every await.
use super::*;
use crate::chains::evm::{explorer, rpc};
use crate::chains::{JournalRow, SendJournal};
use crate::core::whitelist::{self, Fingerprint};
use crate::history::{History, State};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryTarget {
    pub generation: u64,
    pub key: String,
    pub endpoint: RpcChoice,
    pub checked: Option<Fingerprint>,
}
impl HistoryTarget {
    pub fn net(&self) -> Result<Net> {
        Ok(crate::wallet::net_with(
            crate::chains::find(self.endpoint.chain)?,
            &self.endpoint.url,
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryScope {
    pub target: HistoryTarget,
    pub owner: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailRequest {
    pub scope: HistoryScope,
    pub request: u64,
    pub hash: String,
    pub journal: Option<JournalRow>,
    pub explorer_listed: bool,
}
#[derive(Clone, Debug)]
pub struct HistoryRead {
    pub history: History,
    pub list_unchanged: bool,
}
#[derive(Clone, Debug)]
pub struct DetailRead {
    pub detail: TransactionDetail,
    pub list_unchanged: bool,
}
pub use crate::history::tron::Detail as TronDetail;
#[derive(Clone, Debug)]
pub enum TransactionDetail {
    Evm(rpc::TxDetail),
    Tron(TronDetail),
}
impl TransactionDetail {
    pub fn chain(&self) -> &'static str {
        match self {
            Self::Evm(d) => d.chain,
            Self::Tron(d) => d.chain,
        }
    }
    pub fn hash(&self) -> &str {
        match self {
            Self::Evm(d) => &d.tx_hash,
            Self::Tron(d) => &d.tx_hash,
        }
    }
}
pub struct HistoryView {
    pub scope: Option<HistoryScope>,
    pub rows: Pane<History>,
    /// Shown journal rows whose on-chain lookup has not landed (lowercase hashes).
    checking: std::collections::BTreeSet<String>,
    pub selected: usize,
    pub detail: Option<(DetailRequest, Pane<TransactionDetail>)>,
    pub scroll: usize,
    pub balance_focus: bool,
    labels: Option<Fingerprint>,
    request: u64,
}
impl Default for HistoryView {
    fn default() -> Self {
        Self {
            scope: None,
            rows: Pane::Loading,
            checking: Default::default(),
            selected: 0,
            detail: None,
            scroll: 0,
            balance_focus: false,
            labels: None,
            request: 0,
        }
    }
}
/// The list shows at most this many rows, and looks up at most these.
pub const SHOWN_ROWS: usize = 10;
impl HistoryView {
    /// Whether this row's on-chain lookup is still running: the list says `checking`.
    pub fn checking(&self, hash: &str) -> bool {
        self.checking.contains(&hash.to_ascii_lowercase())
    }
}
impl App {
    pub fn history(&self) -> &HistoryView {
        &self.history
    }
    fn target_matches(&self, target: &HistoryTarget) -> bool {
        target.generation == self.generation
            && self.open_key.as_ref() == Some(&target.key)
            && target.endpoint.chain == self.chain.key
            && target.endpoint.url == self.net().rpc_url
            && self.nav.tab == Tab::Wallet
            && !self.nav.every
    }
    pub(super) fn load_history_owner(&mut self) -> Vec<Effect> {
        self.history = HistoryView::default();
        let Some(key) = self.open_key.clone() else {
            return Vec::new();
        };
        if self.nav.tab != Tab::Wallet || self.nav.every {
            return Vec::new();
        }
        vec![Effect::HistoryOwner(HistoryTarget {
            generation: self.generation,
            key,
            endpoint: RpcChoice {
                chain: self.chain.key,
                url: self.net().rpc_url.clone(),
            },
            checked: self.checked_list.as_ref().map(|list| list.fingerprint),
        })]
    }
    pub(super) fn history_update(&mut self, msg: Msg) -> std::ops::ControlFlow<Msg, Vec<Effect>> {
        let work = match msg {
            Msg::HistoryOwner(target, result) => {
                if !self.target_matches(&target) {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                match result.and_then(|(owner, trust)| {
                    self.observe_family_address(&target.key, self.chain, &owner, trust)?;
                    Ok(owner)
                }) {
                    Ok(owner) => {
                        let scope = HistoryScope { target, owner };
                        self.history.scope = Some(scope.clone());
                        vec![Effect::LoadHistory(scope)]
                    }
                    Err(error) => {
                        self.history.rows = Pane::Failed(Outside::error(&error));
                        Vec::new()
                    }
                }
            }
            Msg::History(scope, result) => {
                if !self.target_matches(&scope.target)
                    || self.history.scope.as_ref() != Some(&scope)
                {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                self.history.checking.clear();
                let mut lookups = Vec::new();
                self.history.rows = match result {
                    Ok(read) => {
                        self.accept_labels(scope.target.checked, read.list_unchanged);
                        // A journal row is a local claim: each shown one is looked up on chain
                        // before it shows a status (.prd/03-tui.md §History, source 1).
                        for row in read.history.rows.iter().take(SHOWN_ROWS).filter(|row| {
                            row.state == State::Unknown
                                && row.sources.contains(&crate::history::Source::Journal)
                        }) {
                            self.history.request += 1;
                            self.history
                                .checking
                                .insert(row.tx_hash.to_ascii_lowercase());
                            lookups.push(Effect::LookupHistoryRow(Box::new(DetailRequest {
                                scope: scope.clone(),
                                request: self.history.request,
                                hash: row.tx_hash.clone(),
                                journal: row.journal.clone(),
                                explorer_listed: false,
                            })));
                        }
                        Pane::Ready(read.history)
                    }
                    Err(error) => Pane::Failed(Outside::error(&error)),
                };
                lookups
            }
            Msg::HistoryRow(request, result) => {
                let request = *request;
                let hash = request.hash.to_ascii_lowercase();
                if !self.target_matches(&request.scope.target)
                    || self.history.scope.as_ref() != Some(&request.scope)
                    || !self.history.checking.remove(&hash)
                {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                // A failed lookup leaves the row `unknown`; the detail (Enter) says why.
                if let (Ok(read), Pane::Ready(history)) = (result, &mut self.history.rows)
                    && read.detail.chain() == request.scope.target.endpoint.chain
                    && read.detail.hash().eq_ignore_ascii_case(&hash)
                    && let Some(row) = history
                        .rows
                        .iter_mut()
                        .find(|row| row.tx_hash.eq_ignore_ascii_case(&hash))
                {
                    (row.state, row.success) = match &read.detail {
                        TransactionDetail::Evm(d) => (d.state, d.success),
                        TransactionDetail::Tron(d) => (d.state, d.success),
                    };
                }
                Vec::new()
            }
            Msg::Transaction(request, result) => {
                let request = *request;
                if !self.target_matches(&request.scope.target)
                    || self.history.scope.as_ref() != Some(&request.scope)
                    || !self
                        .history
                        .detail
                        .as_ref()
                        .is_some_and(|(current, _)| current == &request)
                {
                    return std::ops::ControlFlow::Continue(Vec::new());
                }
                let pane = match result {
                    Ok(read)
                        if read.detail.chain() == request.scope.target.endpoint.chain
                            && read.detail.hash().eq_ignore_ascii_case(&request.hash) =>
                    {
                        self.accept_labels(request.scope.target.checked, read.list_unchanged);
                        Pane::Ready(read.detail)
                    }
                    Ok(_) => Pane::Failed(Outside::error(&WalletError::rpc(
                        "transaction reply belongs to another chain or hash",
                    ))),
                    Err(error) => Pane::Failed(Outside::error(&error)),
                };
                self.history.detail = Some((request, pane));
                self.prepare_detail_replacement()
            }
            Msg::HistoryList(generation, fingerprint, result) => {
                if generation == self.generation {
                    self.accept_labels(Some(fingerprint), result.is_ok());
                }
                Vec::new()
            }
            other => return std::ops::ControlFlow::Break(other),
        };
        std::ops::ControlFlow::Continue(work)
    }
    fn accept_labels(&mut self, checked: Option<Fingerprint>, unchanged: bool) {
        if self.checked_list.as_ref().map(|list| list.fingerprint) != checked {
            return;
        }
        self.history.labels = if unchanged { checked } else { None };
        if !unchanged {
            self.checked_list = None;
        }
    }
    pub(super) fn history_tick(&mut self) -> Vec<Effect> {
        self.history.labels = None;
        self.checked_list
            .as_ref()
            .map(|list| vec![Effect::CheckHistoryList(self.generation, list.fingerprint)])
            .unwrap_or_default()
    }
    /// A nickname is a checked-list annotation, never proof about the node's sender.
    pub fn history_nickname(&self, address: &str) -> Option<&str> {
        let list = self.checked_list.as_ref()?;
        if self.history.labels != Some(list.fingerprint) {
            return None;
        }
        list.list
            .entries
            .iter()
            .find(|entry| {
                entry.covers_chain(self.chain.key)
                    && if self.chain.family.name() == "evm" {
                        entry.address.eq_ignore_ascii_case(address)
                    } else {
                        entry.chains == [self.chain.key]
                            && crate::chains::tron::address::parse(address).is_ok_and(|payload| {
                                entry.address.eq_ignore_ascii_case(&format!(
                                    "0x{}",
                                    alloy::hex::encode(&payload[1..])
                                ))
                            })
                    }
            })
            .map(|entry| entry.nickname.as_str())
    }
    pub(super) fn history_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        if self.nav.tab != Tab::Wallet || self.nav.every {
            return None;
        }
        if let Some((request, pane)) = &self.history.detail {
            match key {
                Key::Esc => {
                    self.history.detail = None;
                    return Some(Vec::new());
                }
                Key::Up | Key::Char('k') => {
                    self.history.scroll = self.history.scroll.saturating_sub(1)
                }
                Key::Down | Key::Char('j') => {
                    let count = detail_lines(self, request, pane).len();
                    self.history.scroll = (self.history.scroll + 1).min(count.saturating_sub(1));
                }
                Key::Char('r') => {
                    let hash = request.hash.clone();
                    return Some(self.open_transaction(&hash));
                }
                _ => return None,
            }
            return Some(Vec::new());
        }
        if key == Key::Tab {
            self.history.balance_focus = !self.history.balance_focus;
            return Some(Vec::new());
        }
        if self.history.balance_focus {
            let count = match &self.nav.lines {
                Pane::Ready(lines) => lines.len(),
                _ => 0,
            };
            match key {
                Key::Up | Key::Char('k') => self.nav.scroll = self.nav.scroll.saturating_sub(1),
                Key::Down | Key::Char('j') => {
                    self.nav.scroll = (self.nav.scroll + 1).min(count.saturating_sub(1))
                }
                _ => return None,
            }
            return Some(Vec::new());
        }
        match key {
            Key::Up | Key::Char('k') => {
                self.history.selected = self.history.selected.saturating_sub(1)
            }
            Key::Down | Key::Char('j') => {
                let count = match &self.history.rows {
                    Pane::Ready(h) => h.rows.len(),
                    _ => 0,
                };
                self.history.selected = (self.history.selected + 1).min(count.saturating_sub(1));
            }
            Key::Enter => {
                let hash = match &self.history.rows {
                    Pane::Ready(h) => h.rows.get(self.history.selected).map(|r| r.tx_hash.clone()),
                    _ => None,
                };
                return Some(
                    hash.map(|hash| self.open_transaction(&hash))
                        .unwrap_or_default(),
                );
            }
            _ => return None,
        }
        Some(Vec::new())
    }
    pub(super) fn open_transaction(&mut self, hash: &str) -> Vec<Effect> {
        let Some(scope) = self.history.scope.clone() else {
            self.notice = Some(Outside::detail(
                "open a key and wait for its address before looking up a transaction",
            ));
            return Vec::new();
        };
        if !self.target_matches(&scope.target) {
            return Vec::new();
        }
        let hash = match parse_hash(self.chain, hash) {
            Ok(hash) => hash,
            Err(error) => {
                self.notice = Some(Outside::error(&error));
                return Vec::new();
            }
        };
        let row = match &self.history.rows {
            Pane::Ready(h) => h
                .rows
                .iter()
                .find(|r| r.tx_hash.eq_ignore_ascii_case(&hash)),
            _ => None,
        };
        self.history.request += 1;
        let request = DetailRequest {
            scope,
            request: self.history.request,
            hash,
            journal: row.and_then(|r| r.journal.clone()),
            explorer_listed: row
                .is_some_and(|r| r.sources.contains(&crate::history::Source::Explorer)),
        };
        self.history.detail = Some((request.clone(), Pane::Loading));
        self.history.scroll = 0;
        vec![Effect::LoadTransaction(Box::new(request))]
    }
}
fn list_unchanged(home: &Path, scope: &HistoryScope) -> bool {
    scope
        .target
        .checked
        .is_some_and(|fp| whitelist::check_unchanged(home, &fp).is_ok())
}
fn parse_hash(chain: &crate::chains::Chain, hash: &str) -> Result<String> {
    match chain.family {
        crate::chains::Family::Evm(_) => {
            crate::chains::evm::parse_tx_hash(hash).map(|h| format!("{h:#x}"))
        }
        crate::chains::Family::Tron(_) => crate::chains::tron::lookup::parse_hash(hash),
    }
}
fn validate_owner(chain: &crate::chains::Chain, owner: &str) -> Result<()> {
    match chain.family {
        crate::chains::Family::Evm(_) => crate::chains::evm::parse_address(owner).map(|_| ()),
        crate::chains::Family::Tron(_) => crate::chains::tron::address::parse(owner).map(|_| ()),
    }
}
async fn check_owner(home: &Path, scope: &HistoryScope, net: &Net) -> Result<()> {
    validate_owner(net.chain, &scope.owner)?;
    let (owner, _) = crate::wallet::address(home, &scope.target.key, net.chain).await?;
    if owner != scope.owner {
        return Err(WalletError::validation(
            "selected key's address changed during the history read; verify it again",
        ));
    }
    Ok(())
}
pub async fn read_history(home: &Path, scope: &HistoryScope) -> Result<HistoryRead> {
    let net = scope.target.net()?;
    check_owner(home, scope, &net).await?;
    let journal = SendJournal::new(home, net.chain, &scope.owner).recent()?;
    let page = match net.chain.family {
        crate::chains::Family::Evm(_) => {
            explorer::fetch(net.chain, crate::chains::evm::parse_address(&scope.owner)?).await
        }
        // No Tron indexer is integrated. Do not contact an EVM explorer or imply incoming coverage.
        crate::chains::Family::Tron(_) => Err(explorer::Unavailable::Unsupported),
    };
    Ok(HistoryRead {
        history: crate::history::merge(net.chain, journal, page),
        list_unchanged: list_unchanged(home, scope),
    })
}
pub async fn read_transaction(home: &Path, request: &DetailRequest) -> Result<DetailRead> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let net = request.scope.target.net()?;
        check_owner(home, &request.scope, &net).await?;
        let hash = parse_hash(net.chain, &request.hash)?;
        // Re-read this owner's journal; a stale/caller-supplied row cannot supply the nonce.
        let journal = SendJournal::new(home, net.chain, &request.scope.owner).recent()?;
        let saved = journal
            .iter()
            .find(|j| j.tx_hash.eq_ignore_ascii_case(&hash));
        let detail = match net.chain.family {
            crate::chains::Family::Evm(_) => {
                let owner = crate::chains::evm::parse_address(&request.scope.owner)?;
                let hash = crate::chains::evm::parse_tx_hash(&hash)?;
                let provider = rpc::connect(&net).await?;
                TransactionDetail::Evm(rpc::tx_detail(&provider, &net, hash, owner, saved).await?)
            }
            crate::chains::Family::Tron(_) => TransactionDetail::Tron(
                crate::history::tron::lookup(&net, &hash, &request.scope.owner, saved).await?,
            ),
        };
        Ok(DetailRead {
            detail,
            list_unchanged: list_unchanged(home, &request.scope),
        })
    })
    .await
    .map_err(|_| WalletError::rpc("transaction lookup timed out after 8 seconds"))?
}

pub fn state_label(state: State, success: Option<bool>) -> &'static str {
    match state {
        State::Unknown => "unknown",
        State::Pending => "pending",
        State::NotFound => "not found",
        State::DroppedReplaced => "dropped/replaced",
        State::ExpiredUnseen => "expired-unseen",
        State::Mined => match success {
            Some(true) => "mined (success)",
            Some(false) => "mined (reverted)",
            None => "mined (outcome unknown)",
        },
    }
}
pub fn detail_lines(
    app: &App,
    request: &DetailRequest,
    pane: &Pane<TransactionDetail>,
) -> Vec<Outside> {
    use crate::core::amount::format_units;
    let mut lines = vec![
        Outside::detail(&format!("hash: {}", request.hash)),
        Outside::detail("Source: this node's claim; sender is not a verified signing identity"),
    ];
    let Pane::Ready(detail) = pane else {
        return lines;
    };
    if let Some(note) = app.history_replacement_note() {
        for chunk in note.as_bytes().chunks(72) {
            lines.push(Outside::detail(&String::from_utf8_lossy(chunk)));
        }
    }
    let mut push = |line: String| lines.push(Outside::detail(&line));
    match detail {
        TransactionDetail::Evm(d) => {
            push(format!("Status: {}", state_label(d.state, d.success)));
            if request.explorer_listed && d.state == State::NotFound {
                push("the explorer lists it; this node does not know it".into());
            }
            for (label, value) in [("from", &d.from), ("to", &d.to)] {
                push(format!(
                    "{label}: {}",
                    value
                        .as_deref()
                        .unwrap_or(if label == "to" && d.contract_creation {
                            "contract creation"
                        } else {
                            "unknown"
                        })
                ));
                if let Some(name) = value.as_deref().and_then(|a| app.history_nickname(a)) {
                    push(format!("checked-list nickname: {name}"));
                }
            }
            if let Some(nonce) = d.nonce {
                push(format!("Nonce: {nonce}"));
            } else if let Some(nonce) = d.journal_nonce {
                push(format!("Journal nonce (local claim): {nonce}"));
            }
            let amount = |v: Option<alloy::primitives::U256>| {
                v.map(|n| {
                    format!(
                        "{} {}",
                        format_units(n, app.chain.decimals),
                        app.chain.symbol
                    )
                })
                .unwrap_or_else(|| "unknown".into())
            };
            push(format!("Value: {}", amount(d.value)));
            push(format!("Fee: {}", amount(d.fee)));
            if let Some(l1) = d.l1_fee {
                push(format!("Included L1 fee: {}", amount(Some(l1))));
            }
            if let Some(gas) = d.gas_used {
                push(format!("Gas used: {gas}"));
            }
            if let Some(price) = d.effective_gas_price {
                push(format!("Effective gas price: {price} wei"));
            }
            push(format!(
                "Confirmations: {}",
                d.confirmations
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ));
            if let Some(block) = d.block_number {
                push(format!("Block: {block}"));
            }
            push(d.movements_state.note().into());
            for m in &d.movements {
                push(format!(
                    "{} {}",
                    format_units(m.amount, m.decimals),
                    m.symbol
                ));
                push(format!("  from: {}", m.from));
                push(format!("  to: {}", m.to));
            }
            if d.hidden_movements > 0 {
                push(format!(
                    "{} token movements not shown: outside registry/shape",
                    d.hidden_movements
                ));
            }
            if !d.hint.is_empty() {
                for chunk in d.hint.as_bytes().chunks(72) {
                    push(String::from_utf8_lossy(chunk).into_owned());
                }
            }
        }
        TransactionDetail::Tron(d) => {
            let status = if d.state == State::Mined && d.success == Some(false) {
                "mined (failed)"
            } else {
                state_label(d.state, d.success)
            };
            push(format!("Status: {status}"));
            for (label, address) in [("from", &d.from), ("to", &d.to)] {
                push(format!(
                    "{label}: {}",
                    address.as_deref().unwrap_or("unknown")
                ));
                if let Some(name) = address.as_deref().and_then(|a| app.history_nickname(a)) {
                    push(format!("checked-list nickname: {name}"));
                }
            }
            if let Some(value) = &d.value {
                push(format!("Decoded transfer intent: {value}"));
            }
            if let Some(token) = d.token {
                push(format!("Token: {token}"));
            }
            if let Some(fee) = d.fee_sun {
                push(format!("Fee: {fee} sun"));
            }
            if let Some(block) = d.block {
                push(format!("Block: {block}"));
            }
            if let Some(expiry) = d.expiration_ms {
                push(format!(
                    "Expires: {}",
                    crate::chains::tron::send::expires_at(expiry)
                ));
            }
            if let Some(energy) = d.energy_used {
                push(format!("Energy used: {energy}"));
            }
            if let Some(net) = d.net_usage {
                push(format!("Bandwidth used: {net}"));
            }
            if let Some(at) = &d.as_of {
                push(format!("Not seen as of block {}", at.block));
                push(format!(
                    "Observed at: {}",
                    crate::chains::tron::send::expires_at(at.block_time_ms)
                ));
                push(format!("Node: {}", at.host));
            }
            if let Some(behind) = d.node_behind_secs {
                push(format!("Node behind: {behind} seconds; ask another node"));
            }
            push("Confirmations: unknown (this lookup does not count confirmations)".into());
            if !d.hint.is_empty() {
                for chunk in d.hint.as_bytes().chunks(72) {
                    push(String::from_utf8_lossy(chunk).into_owned());
                }
            }
            push("Sending again can be a second payment.".into());
            push("Absence at this node is not proof of non-payment.".into());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nicknames_require_the_checked_fingerprint_and_same_generation_rewrites_drop_them() {
        use crate::core::whitelist::{Entry, ListFiles, Whitelist};
        let home = tempfile::Builder::new()
            .prefix("zwt-history-list-")
            .tempdir_in("/tmp")
            .unwrap();
        let file = home.path().join("whitelist.json");
        std::fs::write(&file, br#"{"gen":4,"marker":"before"}"#).unwrap();
        let fingerprint = ListFiles::read(home.path()).fingerprint();
        let chain = crate::chains::find("ethereum").unwrap();
        let mut app = App::new(
            &Net {
                chain,
                rpc_url: "http://127.0.0.1:1".into(),
            },
            Duration::from_secs(60),
        );
        let address = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
        let mut list = Whitelist::empty(&crate::chains::evm::registry::WHITELIST_RULES);
        list.entries
            .push(Entry::new_address("alice", address, &["ethereum"]));
        app.checked_list = Some(CheckedList {
            list,
            source: "whitelist.json",
            fingerprint,
        });
        assert_eq!(app.history_nickname(address), None);
        app.accept_labels(Some(fingerprint), true);
        assert_eq!(app.history_nickname(address), Some("alice"));
        std::fs::write(&file, br#"{"gen":4,"marker":"lookalike"}"#).unwrap();
        assert!(matches!(
            app.history_tick().as_slice(),
            [Effect::CheckHistoryList(..)]
        ));
        assert_eq!(app.history_nickname(address), None);
        app.update(Msg::HistoryList(
            app.generation,
            fingerprint,
            whitelist::check_unchanged(home.path(), &fingerprint),
        ));
        assert!(app.checked_list.is_none());
        assert_eq!(app.history_nickname(address), None);
    }
    #[test]
    fn tron_nicknames_use_only_the_checked_tron_scoped_body_and_exact_base58_address() {
        use crate::core::whitelist::{Entry, ListFiles, Whitelist};
        let home = tempfile::Builder::new()
            .prefix("zwt-history-tron-list-")
            .tempdir_in("/tmp")
            .unwrap();
        let fingerprint = ListFiles::read(home.path()).fingerprint();
        let mut app = App::new(
            &Net {
                chain: crate::chains::find("tron").unwrap(),
                rpc_url: "http://127.0.0.1:1".into(),
            },
            Duration::from_secs(60),
        );
        let address = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
        let payload = crate::chains::tron::address::parse(address).unwrap();
        let body = format!("0x{}", alloy::hex::encode(&payload[1..]));
        let mut list = Whitelist::empty(&crate::chains::WHITELIST_RULES);
        list.entries
            .push(Entry::new_address("alice", &body, &["tron"]));
        app.checked_list = Some(CheckedList {
            list,
            source: "whitelist.json",
            fingerprint,
        });
        assert_eq!(app.history_nickname(address), None);
        app.accept_labels(Some(fingerprint), true);
        assert_eq!(app.history_nickname(address), Some("alice"));
        assert_eq!(app.history_nickname(&address.to_ascii_lowercase()), None);
        assert_eq!(app.history_nickname(&body), None);
        app.checked_list.as_mut().unwrap().list.entries[0]
            .chains
            .push("ethereum".into());
        assert_eq!(app.history_nickname(address), None);
        app.accept_labels(Some(fingerprint), false);
        assert!(app.checked_list.is_none());
    }
}

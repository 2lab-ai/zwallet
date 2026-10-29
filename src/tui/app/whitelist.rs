//! Whitelist forms and their ordered, password-backed edit state. No RPC or signing effect.
use super::*;
use crate::core::whitelist::{EntryKind, Fingerprint};
use crate::tui::actions::ActionId;
use crate::tui::form::{Edit, Field, Input, Kind};
use crate::wallet::{self, WhitelistOutcome};
use std::fmt;
use std::path::{Path, PathBuf};

const LIST_CHANGED: &str = "list changed; cancel and verify it again";

pub const ACTIONS: &[ActionId] = &[
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
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Draft {
    Add {
        nickname: String,
        address: String,
        chains: String,
        contract: bool,
        selectors: String,
    },
    Remove(String),
    Fee {
        chain: String,
        value: String,
    },
    Native {
        chain: String,
        value: String,
    },
    Token {
        chain: String,
        token: String,
        value: String,
    },
    Calldata {
        chain: String,
        value: String,
    },
    Message(bool),
    Reset,
}
impl Draft {
    pub fn change(&self) -> Result<wallet::Change> {
        match self {
            Self::Add {
                nickname,
                address,
                chains,
                contract,
                selectors,
            } => {
                let chains: Vec<_> = if chains.is_empty() {
                    Vec::new()
                } else {
                    chains.split(',').collect()
                };
                let selectors: Vec<_> = if selectors.is_empty() {
                    Vec::new()
                } else {
                    selectors.split(',').collect()
                };
                wallet::Change::add(
                    nickname,
                    address,
                    &chains,
                    if *contract {
                        EntryKind::Contract
                    } else {
                        EntryKind::Address
                    },
                    &selectors,
                )
            }
            Self::Remove(nickname) => wallet::Change::remove(nickname),
            Self::Fee { chain, value } => wallet::Change::fee_cap(chain, value),
            Self::Native { chain, value } => wallet::Change::native_cap(chain, value),
            Self::Token {
                chain,
                token,
                value,
            } => wallet::Change::token_cap(chain, token, value),
            Self::Calldata { chain, value } => wallet::Change::calldata_cap(chain, value),
            Self::Message(on) => Ok(wallet::Change::message_signing(*on)),
            Self::Reset => Ok(wallet::Change::reset()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub id: u64,
    pub draft: Draft,
    pub unlock: String,
    /// Selection/toggle must still name the list this TUI verified.
    pub checked_fingerprint: Option<Fingerprint>,
}
#[derive(Debug)]
pub struct Read {
    pub checked: Option<CheckedList>,
    pub text: String,
    pub reason: Option<String>,
}
/// The prepare worker: the plan for `request`, which stops and writes nothing. Display its
/// `summary()` verbatim before asking for the edit's password.
pub fn prepare(
    home: &Path,
    request: EditRequest,
    allow_ptrace_scope_0: bool,
) -> Result<wallet::WhitelistPlan> {
    prepare_with(home, request, allow_ptrace_scope_0, || {})
}

// The plan holds the list files as it read them, and its summary is what the password will
// authorize, so it is compared with the list this TUI verified: one compare on the plan's own
// bytes, after the read, instead of a check before it. `before_read` runs just before that read:
// the test seam for another process of this user rewriting the list.
fn prepare_with(
    home: &Path,
    request: EditRequest,
    allow_ptrace_scope_0: bool,
    before_read: impl FnOnce(),
) -> Result<wallet::WhitelistPlan> {
    let change = request.draft.change()?;
    let unlock = (!request.unlock.is_empty()).then_some(wallet::WhitelistUnlock {
        duration: request.unlock,
        allow_ptrace_scope_0,
    });
    before_read();
    let plan = wallet::whitelist_plan(home, change, unlock)?;
    if request
        .checked_fingerprint
        .is_some_and(|checked| plan.fingerprint() != checked)
    {
        return Err(WalletError::validation(LIST_CHANGED));
    }
    Ok(plan)
}

pub fn read(home: &Path, checked: Option<CheckedList>) -> Result<Read> {
    let mut reason = None;
    if let Some(checked) = checked {
        match wallet::whitelist::checked_view(home, &checked) {
            Ok(view) => {
                return Ok(Read {
                    checked: view.checked,
                    text: view.text,
                    reason,
                });
            }
            Err(error) => reason = Some(error.message),
        }
    }
    let view = wallet::whitelist_view(home, None)?;
    let text = if view.output["source"].as_str() == Some("whitelist.json")
        && view.output["policy"].is_null()
    {
        let why = view.output["reason"].as_str().unwrap_or("invalid list");
        format!(
            "not verified\nwhitelist cannot be read: {}",
            crate::core::text::ascii_inert(why, 80)
        )
    } else {
        view.text
    };
    Ok(Read {
        checked: None,
        text,
        reason,
    })
}
/// Formatting messages never formats an edit result's complete payload.
pub struct Applied(pub Result<WhitelistOutcome>);
impl fmt::Debug for Applied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WhitelistApplied")
            .field("ok", &self.0.is_ok())
            .finish()
    }
}

pub struct WhitelistUi {
    pub stage: Stage,
    pub lines: Pane<Vec<Outside>>,
    pub selected: usize,
    pub scroll: usize,
    pub next_unlock: String,
    pub request: u64,
    // A read may restore cached authority only in the trust generation that requested it.
    read_generation: Option<u64>,
    pub home: PathBuf,
}
impl Default for WhitelistUi {
    fn default() -> Self {
        Self {
            stage: Stage::List,
            lines: Pane::Loading,
            selected: 0,
            scroll: 0,
            next_unlock: String::new(),
            request: 0,
            read_generation: None,
            home: PathBuf::new(),
        }
    }
}
pub enum Stage {
    List,
    Form(Form),
    Preparing,
    Review(Review),
    Applying(Review),
    Outcome { lines: Vec<Outside>, scroll: usize },
}
pub struct Review {
    /// Exact immutable bytes supplied by the same prepared plan the worker will consume.
    pub summary: String,
    pub lines: Vec<Outside>,
    pub scroll: usize,
    /// How many leading rows have all been on screen. Enter opens the password only once this
    /// covers every row.
    seen: usize,
}
/// Rows the review moves by and counts as drawn; an 80x24 frame draws more (16).
const PAGE: usize = 10;
impl Review {
    pub fn new(summary: String) -> Self {
        let lines = lines(&summary);
        Self {
            seen: lines.len().min(PAGE),
            lines,
            summary,
            scroll: 0,
        }
    }
    /// Shows the page from `scroll`. The seen prefix grows only when that page starts inside it,
    /// so rows a move jumped over are never counted.
    fn show(&mut self, scroll: usize) {
        self.scroll = scroll;
        if scroll <= self.seen {
            self.seen = self.seen.max((scroll + PAGE).min(self.lines.len()));
        }
    }
    fn last_page(&self) -> usize {
        self.lines.len().saturating_sub(PAGE)
    }
}
pub struct Form {
    pub action: ActionId,
    pub title: &'static str,
    pub rows: Vec<FormRow>,
    pub selected: usize,
    pub error: Option<Outside>,
    pub remove: Option<String>,
}
pub struct FormRow {
    pub label: &'static str,
    pub field: Field,
    pub optional: bool,
    alphabet: Alphabet,
}
#[derive(Clone, Copy)]
enum Alphabet {
    Field,
    Chain,
    Chains,
    Address,
    Selectors,
    Toggle,
}
impl FormRow {
    fn new(
        label: &'static str,
        kind: Kind,
        value: &str,
        optional: bool,
        alphabet: Alphabet,
    ) -> Self {
        Self {
            label,
            field: Field::with_value(kind, value).expect("trusted initial form value"),
            optional,
            alphabet,
        }
    }
    fn accepts(&self, text: &str) -> bool {
        match self.alphabet {
            Alphabet::Field => true,
            Alphabet::Chain => {
                text.len() <= 32
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            }
            Alphabet::Chains => {
                text.len() <= 128
                    && text.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"-,*".contains(&b)
                    })
            }
            Alphabet::Address => {
                text.len() <= 42 && text.bytes().all(|b| b.is_ascii_alphanumeric())
            }
            Alphabet::Selectors => {
                text.len() <= 175
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() || b"x,".contains(&b))
            }
            Alphabet::Toggle => "on".starts_with(text) || "off".starts_with(text),
        }
    }
    fn edit(&mut self, input: Input<'_>) -> Edit {
        let added = match input {
            Input::Char(c) => Some(c.to_string()),
            Input::Paste(s) => Some(s.to_string()),
            _ => None,
        };
        if let Some(added) = added {
            let mut next = self.field.value().to_string();
            next.insert_str(self.field.cursor(), &added);
            if !self.accepts(&next) {
                return Edit::Rejected("outside this field's alphabet or length limit");
            }
        }
        if matches!(input, Input::Enter) && self.optional && self.field.value().is_empty() {
            return Edit::Submit;
        }
        if matches!(self.alphabet, Alphabet::Toggle)
            && matches!(input, Input::Left | Input::Right | Input::Char(' '))
        {
            self.field = Field::with_value(
                Kind::Filter,
                if self.field.value() == "on" {
                    "off"
                } else {
                    "on"
                },
            )
            .expect("toggle");
            return Edit::Changed;
        }
        self.field.edit(input)
    }
}
impl Form {
    fn value(&self, label: &str) -> &str {
        self.rows
            .iter()
            .find(|r| r.label == label)
            .map(|r| r.field.value())
            .unwrap_or_default()
    }
    fn draft(&self) -> Result<Draft> {
        let text = |label| self.value(label).to_string();
        Ok(match self.action {
            ActionId::WhitelistAdd | ActionId::WhitelistContract => Draft::Add {
                nickname: text("nickname"),
                address: text("address"),
                chains: text("chains"),
                contract: self.action == ActionId::WhitelistContract,
                selectors: text("selectors"),
            },
            ActionId::WhitelistRemove => {
                Draft::Remove(self.remove.clone().expect("selected verified entry"))
            }
            ActionId::FeeCap => Draft::Fee {
                chain: text("chain"),
                value: text("coin amount"),
            },
            ActionId::NativeCap => Draft::Native {
                chain: text("chain"),
                value: text("coin amount or none"),
            },
            ActionId::TokenCap => Draft::Token {
                chain: text("chain"),
                token: text("registry token address"),
                value: text("base units or none"),
            },
            ActionId::CalldataCap => Draft::Calldata {
                chain: text("chain"),
                value: text("bytes or none"),
            },
            ActionId::MessagePolicy => match self.value("message signing") {
                "on" => Draft::Message(true),
                "off" => Draft::Message(false),
                _ => return Err(WalletError::validation("message signing must be on or off")),
            },
            ActionId::Reset => Draft::Reset,
            _ => return Err(WalletError::validation("not a whitelist mutation")),
        })
    }
}
/// A whole bounded ASCII line is retained across rows rather than cut with an ellipsis.
/// The list pane's rows (rule 4): one row per line of the list text, never wrapped, cut to the
/// pane's width when drawn. The fee caps get one row per chain instead of one run-on line, and a
/// chain's per-transaction limits one row per limit instead of one line of JSON.
pub fn list_rows(text: &str) -> Vec<Outside> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if let Some(caps) = line.strip_prefix("fee caps: ") {
            rows.extend(caps.split(", ").map(|cap| {
                Outside::list_row(&if cap.starts_with("and ") {
                    format!("fee caps: {cap}")
                } else {
                    format!("fee cap {cap}")
                })
            }));
        } else if let Some(limits) = limit_rows(line) {
            rows.extend(limits.iter().map(|row| Outside::list_row(row)));
        } else {
            rows.push(Outside::list_row(line));
        }
    }
    rows
}

/// `per-transaction limits (native=<unit>, token=base units): <chain>: <json>`, the list text's
/// line for one chain, as a heading and one row per limit; `None` for any other line (or one
/// whose JSON does not read, which then stays one row, cut).
fn limit_rows(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix("per-transaction limits (")?;
    let (units, rest) = rest.split_once("): ")?;
    let (chain, json) = rest.split_once(": ")?;
    let limits: serde_json::Value = serde_json::from_str(json).ok()?;
    let limits = limits.as_object()?;
    let mut rows = vec![format!("per-transaction limits on {chain} ({units}):")];
    for (key, value) in limits {
        let shown =
            |v: &serde_json::Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
        match (key.as_str(), value) {
            ("maxNativeWei", v) => rows.push(format!("  native {}", shown(v))),
            ("maxCalldataBytes", v) => rows.push(format!("  calldata {} bytes", shown(v))),
            ("maxTokenBaseUnits", serde_json::Value::Object(tokens)) => rows.extend(
                tokens
                    .iter()
                    .map(|(token, cap)| format!("  token {token}: {}", shown(cap))),
            ),
            (other, v) => rows.push(format!("  {other} {}", shown(v))),
        }
    }
    Some(rows)
}

pub fn lines(text: &str) -> Vec<Outside> {
    text.lines()
        .flat_map(|line| {
            if line.is_empty() {
                return vec![Outside::detail("")];
            }
            // Shared summaries are already inert. This also escapes a fabricated worker reply.
            let safe = crate::core::text::ascii_inert(line, 4096);
            safe.as_bytes()
                .chunks(72)
                .map(|part| Outside::detail(std::str::from_utf8(part).expect("ASCII")))
                .collect()
        })
        .collect()
}
impl App {
    pub fn whitelist_ui(&self) -> &WhitelistUi {
        &self.whitelist
    }
    pub fn whitelist_busy(&self) -> bool {
        matches!(self.whitelist.stage, Stage::Applying(_))
    }
    pub(super) fn load_whitelist(&mut self) -> Vec<Effect> {
        self.whitelist.request += 1;
        self.whitelist.read_generation = Some(self.generation);
        let checked = self.checked_list.take();
        self.whitelist.lines = Pane::Loading;
        vec![Effect::ReadWhitelist(self.whitelist.request, checked)]
    }
    fn whitelist_verified(&mut self) -> bool {
        let Some(checked) = self.checked_list.as_ref() else {
            self.notice = Some(Outside::detail(
                "V verifies the list before selecting or toggling its entries",
            ));
            return false;
        };
        // Selection is local only; the worker checks this exact fingerprint before preparing.
        let _ = checked;
        true
    }
    pub(super) fn whitelist_action(&mut self, action: ActionId) -> Option<Vec<Effect>> {
        if !ACTIONS.contains(&action) {
            return None;
        }
        self.nav.tab = Tab::Whitelist;
        self.nav.popup = None;
        self.notice = None;
        if self.locking
            || self.activating
            || self.password != PasswordState::Idle
            || self.whitelist_busy()
        {
            return Some(Vec::new());
        }
        match action {
            ActionId::WhitelistList => {
                self.whitelist.stage = Stage::List;
                return Some(self.load_whitelist());
            }
            ActionId::WhitelistVerify => {
                self.whitelist.stage = Stage::List;
                return Some(self.begin_password(PasswordPurpose::WhitelistVerify));
            }
            _ => {}
        }
        if !matches!(&self.status, Pane::Ready(bar) if bar.configured) {
            self.notice = Some(Outside::detail("needs: a wallet"));
            return Some(Vec::new());
        }
        let mut rows = Vec::new();
        let mut remove = None;
        let field = |label, kind, value, optional| {
            FormRow::new(label, kind, value, optional, Alphabet::Field)
        };
        let chain = || {
            FormRow::new(
                "chain",
                Kind::Filter,
                self.chain.key,
                false,
                Alphabet::Chain,
            )
        };
        let title = match action {
            ActionId::WhitelistAdd | ActionId::WhitelistContract => {
                rows.push(field("nickname", Kind::Nickname, "", false));
                rows.push(FormRow::new(
                    "address",
                    Kind::Signature,
                    "",
                    false,
                    Alphabet::Address,
                ));
                rows.push(FormRow::new(
                    "chains",
                    Kind::Signature,
                    "",
                    true,
                    Alphabet::Chains,
                ));
                if action == ActionId::WhitelistContract {
                    rows.push(FormRow::new(
                        "selectors",
                        Kind::Signature,
                        "",
                        false,
                        Alphabet::Selectors,
                    ));
                }
                if action == ActionId::WhitelistContract {
                    "add contract (EVM only)"
                } else {
                    "add payee"
                }
            }
            ActionId::WhitelistRemove => {
                if !self.whitelist_verified() {
                    return Some(Vec::new());
                }
                remove = self
                    .checked_list
                    .as_ref()
                    .and_then(|l| l.list.entries.get(self.whitelist.selected))
                    .map(|e| e.nickname.clone());
                if remove.is_none() {
                    self.notice = Some(Outside::detail("there is no entry to remove"));
                    return Some(Vec::new());
                }
                "remove selected entry"
            }
            ActionId::FeeCap => {
                rows.extend([chain(), field("coin amount", Kind::Amount, "", false)]);
                "fee cap"
            }
            ActionId::NativeCap => {
                rows.extend([
                    chain(),
                    field("coin amount or none", Kind::LimitAmount, "", false),
                ]);
                "per-transaction native cap"
            }
            ActionId::TokenCap => {
                rows.extend([
                    chain(),
                    FormRow::new(
                        "registry token address",
                        Kind::Signature,
                        "",
                        false,
                        Alphabet::Address,
                    ),
                    field("base units or none", Kind::LimitRaw, "", false),
                ]);
                "per-transaction token cap"
            }
            ActionId::CalldataCap => {
                rows.extend([chain(), field("bytes or none", Kind::LimitBytes, "", false)]);
                "per-transaction calldata cap"
            }
            ActionId::MessagePolicy => {
                if !self.whitelist_verified() {
                    return Some(Vec::new());
                }
                let on = self
                    .checked_list
                    .as_ref()
                    .is_some_and(|l| l.list.policy.allow_message_signing);
                rows.push(FormRow::new(
                    "message signing",
                    Kind::Filter,
                    if on { "off" } else { "on" },
                    false,
                    Alphabet::Toggle,
                ));
                "message signing (left/right toggles)"
            }
            ActionId::Reset => "reset policy and rotate its signing key",
            ActionId::WhitelistUnlock => "unlock duration for the next edit only",
            _ => unreachable!(),
        };
        let duration = std::mem::take(&mut self.whitelist.next_unlock);
        rows.push(field(
            "unlock duration (blank = locked)",
            Kind::Duration,
            &duration,
            true,
        ));
        self.whitelist.stage = Stage::Form(Form {
            action,
            title,
            rows,
            selected: 0,
            error: None,
            remove,
        });
        Some(Vec::new())
    }
    pub(super) fn whitelist_input(&mut self, input: Input<'_>) -> Option<Vec<Effect>> {
        if self.nav.tab != Tab::Whitelist || self.nav.popup.is_some() {
            return None;
        }
        if self.whitelist_busy() {
            self.notice = Some(Outside::detail(
                "saving policy; wait for the result before leaving or locking",
            ));
            return Some(Vec::new());
        }
        // Only a form's text fields keep the tab keys; a review or an outcome stays for the return.
        if super::navigation::switches_tab(&input)
            && !matches!(self.whitelist.stage, Stage::Form(_))
        {
            return None;
        }
        let stage = std::mem::replace(&mut self.whitelist.stage, Stage::List);
        match stage {
            Stage::Form(mut form) => {
                let edit = form.rows[form.selected].edit(input);
                form.error = None;
                match edit {
                    Edit::Cancel => return Some(Vec::new()),
                    Edit::Interrupt => {
                        self.quit = true;
                        return Some(Vec::new());
                    }
                    Edit::Previous => form.selected = form.selected.saturating_sub(1),
                    Edit::Next => form.selected = (form.selected + 1).min(form.rows.len() - 1),
                    Edit::Submit if form.selected + 1 < form.rows.len() => form.selected += 1,
                    Edit::Submit => {
                        if form.action == ActionId::WhitelistUnlock {
                            self.whitelist.next_unlock =
                                form.value("unlock duration (blank = locked)").to_string();
                            self.notice = Some(Outside::detail(
                                "duration set for the next edit; its summary will confirm it",
                            ));
                            return Some(Vec::new());
                        }
                        let draft = form.draft();
                        let valid = draft.and_then(|draft| draft.change().map(|_| draft));
                        match valid {
                            Ok(draft) => {
                                if matches!(draft, Draft::Remove(_) | Draft::Message(_))
                                    && !self.whitelist_verified()
                                {
                                    form.error = Some(Outside::detail(LIST_CHANGED));
                                } else {
                                    self.whitelist.request += 1;
                                    let request = EditRequest {
                                        id: self.whitelist.request,
                                        draft,
                                        unlock: form
                                            .value("unlock duration (blank = locked)")
                                            .to_string(),
                                        checked_fingerprint: self
                                            .checked_list
                                            .as_ref()
                                            .map(|l| l.fingerprint),
                                    };
                                    self.password_purpose = PasswordPurpose::WhitelistEdit;
                                    self.password = PasswordState::Preparing;
                                    self.whitelist.stage = Stage::Preparing;
                                    return Some(vec![Effect::PrepareWhitelist(request)]);
                                }
                            }
                            Err(error) => form.error = Some(Outside::error(&error)),
                        }
                    }
                    Edit::Rejected(reason) => form.error = Some(Outside::detail(reason)),
                    _ => {}
                }
                self.whitelist.stage = Stage::Form(form);
            }
            Stage::Review(mut review) if self.password == PasswordState::Idle => {
                match input {
                    Input::Char('L') => {
                        self.locking = true;
                        return Some(vec![
                            Effect::DiscardWhitelist(self.whitelist.request),
                            Effect::LockAll,
                        ]);
                    }
                    Input::Esc | Input::Interrupt | Input::Char('q') => {
                        if matches!(input, Input::Interrupt | Input::Char('q')) {
                            self.quit = true;
                        }
                        return Some(vec![Effect::DiscardWhitelist(self.whitelist.request)]);
                    }
                    Input::Up => review.show(review.scroll.saturating_sub(1)),
                    Input::Down => {
                        review.show((review.scroll + 1).min(review.lines.len().saturating_sub(1)))
                    }
                    Input::Home => review.show(0),
                    // End pages one screen at a time: a jump would skip rows never drawn.
                    Input::End => review.show((review.scroll + PAGE).min(review.last_page())),
                    // Enter first shows the first rows not yet on screen.
                    Input::Enter if review.seen < review.lines.len() => {
                        review.show(review.seen.min(review.last_page()))
                    }
                    Input::Enter => {
                        self.password_purpose = PasswordPurpose::WhitelistEdit;
                        self.password = PasswordState::Input {
                            characters: 0,
                            rejected: false,
                        };
                        self.pending_key = Some(String::new());
                    }
                    _ => {}
                }
                self.whitelist.stage = Stage::Review(review);
            }
            Stage::Outcome { lines, mut scroll } => {
                match input {
                    Input::Char('L') => {
                        // This is a result view, not a text field or a live mutation. Let
                        // the common panic-button path lock immediately and sequence q.
                        self.whitelist.stage = Stage::Outcome { lines, scroll };
                        return None;
                    }
                    Input::Esc | Input::Enter => return Some(self.load_whitelist()),
                    Input::Interrupt | Input::Char('q') => {
                        self.quit = true;
                        return Some(Vec::new());
                    }
                    Input::Up | Input::Char('k') => scroll = scroll.saturating_sub(1),
                    Input::Down | Input::Char('j') => {
                        scroll = (scroll + 1).min(lines.len().saturating_sub(1))
                    }
                    _ => {}
                }
                self.whitelist.stage = Stage::Outcome { lines, scroll };
            }
            Stage::List => {
                let action = match input {
                    Input::Char('V') => Some(ActionId::WhitelistVerify),
                    Input::Char('a') => Some(ActionId::WhitelistAdd),
                    Input::Char('C') => Some(ActionId::WhitelistContract),
                    Input::Char('d') => Some(ActionId::WhitelistRemove),
                    Input::Char('f') => Some(ActionId::FeeCap),
                    Input::Char('n') => Some(ActionId::NativeCap),
                    Input::Char('t') => Some(ActionId::TokenCap),
                    Input::Char('b') => Some(ActionId::CalldataCap),
                    Input::Char('m') => Some(ActionId::MessagePolicy),
                    Input::Char('R') => Some(ActionId::Reset),
                    Input::Char('u') => Some(ActionId::WhitelistUnlock),
                    Input::Up | Input::Char('k') => {
                        self.whitelist.selected = self.whitelist.selected.saturating_sub(1);
                        self.whitelist.scroll = self.whitelist.scroll.saturating_sub(1);
                        return Some(Vec::new());
                    }
                    Input::Down | Input::Char('j') => {
                        let count = self
                            .checked_list
                            .as_ref()
                            .map_or(0, |l| l.list.entries.len());
                        self.whitelist.selected =
                            (self.whitelist.selected + 1).min(count.saturating_sub(1));
                        let length = match &self.whitelist.lines {
                            Pane::Ready(l) => l.len(),
                            _ => 0,
                        };
                        self.whitelist.scroll =
                            (self.whitelist.scroll + 1).min(length.saturating_sub(1));
                        return Some(Vec::new());
                    }
                    _ => None,
                };
                return action.and_then(|a| self.whitelist_action(a));
            }
            other => {
                self.whitelist.stage = other;
                return None;
            }
        }
        Some(Vec::new())
    }
    pub(super) fn whitelist_update(&mut self, msg: Msg) -> std::ops::ControlFlow<Msg, Vec<Effect>> {
        let work = match msg {
            Msg::WhitelistRead(id, result)
                if id == self.whitelist.request
                    && self.whitelist.read_generation == Some(self.generation) =>
            {
                self.whitelist.read_generation = None;
                match result {
                    Ok(read) => {
                        self.checked_list = read.checked;
                        self.list_reason = read.reason.map(|s| Outside::warning(&s));
                        self.whitelist.lines = Pane::Ready(list_rows(&read.text));
                    }
                    Err(error) => {
                        self.checked_list = None;
                        self.whitelist.lines = Pane::Failed(Outside::error(&error));
                    }
                }
                Vec::new()
            }
            Msg::WhitelistRead(..) => Vec::new(),
            Msg::WhitelistPrepared(id, result)
                if id == self.whitelist.request
                    && matches!(self.whitelist.stage, Stage::Preparing) =>
            {
                self.password = PasswordState::Idle;
                match result {
                    Ok(summary) => self.whitelist.stage = Stage::Review(Review::new(summary)),
                    Err(error) => {
                        self.whitelist.stage = Stage::Outcome {
                            lines: lines(&error.message),
                            scroll: 0,
                        };
                    }
                }
                Vec::new()
            }
            Msg::WhitelistPrepared(..) => Vec::new(),
            Msg::WhitelistApplied(id, Applied(result))
                if id == self.whitelist.request && self.whitelist_busy() =>
            {
                self.password = PasswordState::Idle;
                self.pending_key = None;
                self.checked_list = None;
                let text = match result {
                    Ok(outcome) => {
                        let mut text = outcome.text;
                        match outcome.checked {
                            Ok(checked) => {
                                self.checked_list = Some(checked);
                                self.list_reason = None;
                            }
                            Err(reason) => {
                                self.list_reason = Some(Outside::warning(&reason));
                                text.push_str(&format!("\nnot verified after edit: {reason}"));
                            }
                        }
                        text
                    }
                    Err(error) => {
                        if self.quit {
                            self.quit_error = Some(error.clone());
                        }
                        self.list_reason = Some(Outside::warning(
                            "check the list again after the failed edit",
                        ));
                        error.message
                    }
                };
                self.whitelist.stage = Stage::Outcome {
                    lines: lines(&text),
                    scroll: 0,
                };
                self.invalidate_reads();
                let mut work = self.load_status();
                work.extend(self.load_keys());
                if std::mem::take(&mut self.lock_after_check) {
                    work.push(Effect::LockAll);
                }
                work
            }
            Msg::WhitelistApplied(..) => Vec::new(),
            Msg::PasswordSubmitted
                if self.password_purpose == PasswordPurpose::WhitelistEdit
                    && matches!(self.password, PasswordState::Input { .. }) =>
            {
                let stage = std::mem::replace(&mut self.whitelist.stage, Stage::List);
                if let Stage::Review(review) = stage {
                    self.whitelist.stage = Stage::Applying(review);
                    self.password = PasswordState::Checking;
                    self.checked_list = None;
                    vec![Effect::ApplyWhitelist(self.whitelist.request)]
                } else {
                    self.whitelist.stage = stage;
                    Vec::new()
                }
            }
            Msg::PasswordCancelled
                if self.password_purpose == PasswordPurpose::WhitelistEdit
                    && !self.whitelist_busy() =>
            {
                self.password = PasswordState::Idle;
                self.pending_key = None;
                self.whitelist.stage = Stage::List;
                vec![Effect::DiscardWhitelist(self.whitelist.request)]
            }
            Msg::Key(Key::Char('L')) if self.whitelist_busy() => {
                // Finish the authorized edit and optional start, then honor the panic lock.
                // Stopping now could be undone by the session this worker has yet to start.
                self.locking = true;
                self.lock_after_check = true;
                Vec::new()
            }
            Msg::Key(Key::Char('q') | Key::Interrupt) if self.whitelist_busy() => {
                self.quit = true;
                Vec::new()
            }
            Msg::Key(_) | Msg::Paste(_) | Msg::PasswordSubmitted | Msg::PasswordCancelled
                if self.whitelist_busy() =>
            {
                Vec::new()
            }
            Msg::Tick
                if self.nav.tab == Tab::Whitelist
                    && matches!(self.whitelist.stage, Stage::List)
                    && self.password == PasswordState::Idle =>
            {
                let mut work = self.load_status();
                work.extend(self.load_whitelist());
                work
            }
            Msg::Paste(text)
                if self.nav.tab == Tab::Whitelist
                    && self.password == PasswordState::Idle
                    && self.nav.popup.is_none() =>
            {
                self.whitelist_input(Input::Paste(&text.0))
                    .unwrap_or_default()
            }
            other => return std::ops::ControlFlow::Break(other),
        };
        std::ops::ControlFlow::Continue(work)
    }
}

#[cfg(test)]
#[path = "whitelist_tests.rs"]
mod tests;

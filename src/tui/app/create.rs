//! Creation is one confirmed mutation. Public state never owns password bytes or a phrase.
use super::*;
use crate::core::error::ErrorCode;
use crate::core::time::parse_duration;
use crate::tui::form::{Edit, Field, Input, Kind};
use crate::wallet;
use std::ops::ControlFlow;

pub const BACKUP_HINT: &str = "run zwallet backup in a shell to see the recovery phrase";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateEffect {
    Prepare {
        id: u64,
        name: String,
        words: Option<usize>,
        ttl: Option<Duration>,
    },
    Submit {
        id: u64,
    },
}
#[derive(Debug)]
pub enum CreateMsg {
    Prepared(u64, Result<bool>),
    PasswordAgain,
    PasswordMismatch,
    Finished(u64, Result<wallet::Created>),
}
#[derive(Default)]
pub enum CreateStage {
    #[default]
    Idle,
    Form,
    Preparing,
    PasswordFirst,
    PasswordAgain,
    PasswordExisting,
    Submitting,
    Done(wallet::Created),
}
impl CreateStage {
    pub fn active(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}
pub struct CreateForm {
    pub first: bool,
    pub focus: usize,
    pub fields: [Field; 4],
}
impl CreateForm {
    fn new(first: bool) -> Self {
        Self {
            first,
            focus: 0,
            fields: [
                Field::new(Kind::KeyName),
                Field::with_value(Kind::WordCount, "24").expect("constant word count"),
                Field::new(Kind::Duration),
                Field::new(Kind::KeyName),
            ],
        }
    }
    fn advance(&mut self) {
        self.focus += 1;
        if self.focus == 1 && !self.first {
            self.focus = 2;
        }
    }
    fn back(&mut self) {
        self.focus = self.focus.saturating_sub(1);
        if self.focus == 1 && !self.first {
            self.focus = 0;
        }
    }
}
#[derive(Default)]
pub struct CreateUi {
    pub stage: CreateStage,
    pub form: Option<CreateForm>,
    pub notice: Option<Outside>,
    pub mutation_failed: bool,
    request: u64,
    first_prompted: bool,
}
impl App {
    pub fn creation(&self) -> &CreateUi {
        &self.create
    }
    pub(super) fn begin_create(&mut self) -> Vec<Effect> {
        if self.locking || self.activating || self.password != PasswordState::Idle || self.quit {
            return Vec::new();
        }
        let exists = matches!(&self.status, Pane::Ready(bar) if bar.configured)
            || matches!(&self.keys, Pane::Ready(keys) if !keys.rows.is_empty());
        self.create.first_prompted = true;
        self.create.stage = CreateStage::Form;
        self.create.form = Some(CreateForm::new(!exists));
        self.create.notice = None;
        self.create.mutation_failed = false;
        self.nav.popup = None;
        self.nav.tab = Tab::Keys;
        Vec::new()
    }
    pub(super) fn backup_hint(&mut self) -> Vec<Effect> {
        self.nav.popup = Some(Popup::Lines {
            title: "recovery phrase",
            lines: vec![Outside::detail(BACKUP_HINT)],
        });
        Vec::new()
    }
    pub(super) fn create_update(&mut self, msg: Msg) -> ControlFlow<Msg, Vec<Effect>> {
        use ControlFlow::{Break, Continue};
        let work = match msg {
            Msg::Keys(generation, Ok(_))
            | Msg::Status(
                generation,
                Ok(StatusView {
                    configured: true, ..
                }),
            ) if generation == self.generation => {
                self.create.first_prompted = true;
                return Break(msg);
            }
            Msg::Keys(generation, Err(error))
                if generation == self.generation
                    && error.code == ErrorCode::WalletNotFound
                    && !self.create.first_prompted =>
            {
                self.keys_pending = false;
                self.keys = Pane::Ready(KeyList {
                    rows: vec![],
                    warning: None,
                });
                self.begin_create()
            }
            Msg::Status(generation, Ok(status))
                if generation == self.generation
                    && !status.configured
                    && !self.create.first_prompted =>
            {
                self.create.first_prompted = true;
                let work = self.update(Msg::Status(generation, Ok(status)));
                self.begin_create();
                work
            }
            Msg::Create(CreateMsg::Prepared(id, result)) => {
                if id != self.create.request || !matches!(self.create.stage, CreateStage::Preparing)
                {
                    return Continue(Vec::new());
                }
                if self.quit {
                    self.create.stage = CreateStage::Idle;
                    self.password = PasswordState::Idle;
                    return Continue(Vec::new());
                }
                match result {
                    Ok(first)
                        if self
                            .create
                            .form
                            .as_ref()
                            .is_some_and(|form| form.first != first) =>
                    {
                        self.password = PasswordState::Failed;
                        self.create.stage = CreateStage::Form;
                        self.create.notice = Some(Outside::error(&WalletError::validation(
                            "the wallet appeared or disappeared during preparation; return to keys and start again",
                        )));
                    }
                    Ok(first) => {
                        self.create.stage = if first {
                            CreateStage::PasswordFirst
                        } else {
                            CreateStage::PasswordExisting
                        };
                        self.password = PasswordState::Input {
                            characters: 0,
                            rejected: false,
                        };
                    }
                    Err(error) => {
                        self.password = PasswordState::Failed;
                        self.create.notice = Some(Outside::error(&error));
                        self.create.stage = CreateStage::Form;
                    }
                }
                Vec::new()
            }
            Msg::Create(CreateMsg::PasswordAgain)
                if matches!(self.create.stage, CreateStage::PasswordFirst) =>
            {
                self.create.stage = CreateStage::PasswordAgain;
                self.create.notice = None;
                self.password = PasswordState::Input {
                    characters: 0,
                    rejected: false,
                };
                Vec::new()
            }
            Msg::Create(CreateMsg::PasswordMismatch)
                if matches!(self.create.stage, CreateStage::PasswordAgain) =>
            {
                self.create.stage = CreateStage::PasswordFirst;
                self.create.notice = Some(Outside::detail(
                    "Passwords differ. Both cleared; enter a new password again.",
                ));
                self.password = PasswordState::Input {
                    characters: 0,
                    rejected: false,
                };
                Vec::new()
            }
            Msg::PasswordSubmitted
                if matches!(
                    self.create.stage,
                    CreateStage::PasswordAgain | CreateStage::PasswordExisting
                ) =>
            {
                self.create.stage = CreateStage::Submitting;
                self.create.notice = None;
                self.password = PasswordState::Checking;
                self.invalidate_reads();
                vec![Effect::Create(CreateEffect::Submit {
                    id: self.create.request,
                })]
            }
            Msg::Create(CreateMsg::Finished(id, result)) => {
                if id != self.create.request
                    || !matches!(self.create.stage, CreateStage::Submitting)
                {
                    return Continue(Vec::new());
                }
                self.password = PasswordState::Idle;
                let result = result.and_then(|created| {
                    if !created.verified || created.addresses.is_empty() {
                        return Err(WalletError::validation(
                            "creation did not return locally verified addresses",
                        ));
                    }
                    if self.verified_indices.get(&created.name).is_some_and(|index| *index != created.index) {
                        return Err(WalletError::validation("the created key index conflicts with this session's verified key; close and verify again"));
                    }
                    let mut family_addresses = BTreeMap::new();
                    for (key, address) in &created.addresses {
                        let chain = crate::chains::find(key)?;
                        if let Some(previous) = family_addresses.insert(chain.family.name(),address)
                            && previous != address {
                            return Err(WalletError::validation("the created family addresses disagree"));
                        }
                        self.check_family_pin(&created.name, crate::chains::find(key)?, address)?;
                    }
                    let primary = created
                        .addresses
                        .get(crate::chains::PRIMARY_CHAIN)
                        .ok_or_else(|| {
                            WalletError::validation("created key has no primary address")
                        })?;
                    self.check_pin(&created.name, primary)?;
                    Ok(created)
                });
                let mut work = match result {
                    Ok(created) => {
                        self.verified_indices
                            .entry(created.name.clone())
                            .or_insert(created.index);
                        for (key, address) in &created.addresses {
                            let chain = crate::chains::find(key).expect("checked chain");
                            self.nav.family_pins.insert(
                                (created.name.clone(), chain.family.name()),
                                address.clone(),
                            );
                        }
                        self.pins.insert(
                            created.name.clone(),
                            created.addresses[crate::chains::PRIMARY_CHAIN].clone(),
                        );
                        self.open_key = Some(created.name.clone());
                        self.nav.tab = Tab::Keys;
                        self.create.stage = CreateStage::Done(created);
                        self.create.form = None;
                        self.reload()
                    }
                    Err(error) => {
                        if self.quit {
                            self.quit_error = Some(error.clone());
                        }
                        self.create.stage = CreateStage::Form;
                        self.create.mutation_failed = true;
                        self.create.notice = Some(Outside::error(&error));
                        self.reload()
                    }
                };
                if std::mem::take(&mut self.lock_after_check) {
                    work.push(Effect::LockAll);
                }
                work
            }
            Msg::PasswordSubmitted if self.create.stage.active() => Vec::new(),
            Msg::Create(_) => Vec::new(),
            Msg::PasswordCancelled if matches!(self.create.stage, CreateStage::Submitting) => {
                Vec::new()
            }
            Msg::PasswordCancelled if self.create.stage.active() => {
                self.create.stage = CreateStage::Idle;
                self.create.form = None;
                self.create.notice = None;
                self.password = PasswordState::Idle;
                Vec::new()
            }
            Msg::Paste(text)
                if matches!(self.create.stage, CreateStage::Form)
                    && self.password == PasswordState::Idle =>
            {
                self.create_input(Input::Paste(&text.0))
            }
            Msg::Key(Key::Char('L')) if matches!(self.create.stage, CreateStage::Done(_)) => {
                // The public result is not a focused field. Keep the panic lock available
                // without dismissing the new address or its backup reminder.
                return Break(msg);
            }
            Msg::Key(key) if self.create.stage.active() && self.password == PasswordState::Idle => {
                self.create_input(super::navigation::input(key))
            }
            Msg::Key(Key::Char('n'))
                if self.nav.tab == Tab::Keys
                    && self.nav.popup.is_none()
                    && self.confirm.is_none() =>
            {
                self.begin_create()
            }
            Msg::Key(Key::Char('B'))
                if self.nav.tab == Tab::Keys
                    && self.nav.popup.is_none()
                    && self.password == PasswordState::Idle =>
            {
                self.backup_hint()
            }
            other => return Break(other),
        };
        Continue(work)
    }
    fn create_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        if let Input::Interrupt = input {
            self.quit = true;
            self.create.stage = CreateStage::Idle;
            self.create.form = None;
            return Vec::new();
        }
        if matches!(self.create.stage, CreateStage::Done(_)) {
            match input {
                Input::Char('B') => {
                    self.create.stage = CreateStage::Idle;
                    return self.backup_hint();
                }
                Input::Enter | Input::Esc => self.create.stage = CreateStage::Idle,
                Input::Char('q') => self.quit = true,
                _ => {}
            }
            return Vec::new();
        }
        let Some(form) = &mut self.create.form else {
            return Vec::new();
        };
        self.create.notice = None;
        if form.focus == 1 && matches!(input, Input::Left | Input::Right) {
            let counts = [12, 15, 18, 21, 24];
            let i = counts
                .iter()
                .position(|v| v.to_string() == form.fields[1].value())
                .unwrap_or(4);
            let i = if matches!(input, Input::Left) {
                (i + 4) % 5
            } else {
                (i + 1) % 5
            };
            form.fields[1] = Field::with_value(Kind::WordCount, &counts[i].to_string()).unwrap();
            return Vec::new();
        }
        let edit = if form.focus == 2
            && form.fields[2].value().is_empty()
            && matches!(input, Input::Enter | Input::Tab)
        {
            Edit::Submit
        } else {
            form.fields[form.focus].edit(input)
        };
        match edit {
            Edit::Cancel => {
                self.create.stage = CreateStage::Idle;
                self.create.form = None;
            }
            Edit::Previous => form.back(),
            Edit::Next | Edit::Submit => {
                if !(form.fields[form.focus].is_complete()
                    || form.focus == 2 && form.fields[2].value().is_empty())
                {
                    self.create.notice =
                        Some(Outside::detail("Complete this field before continuing."));
                    return Vec::new();
                }
                if form.focus < 3 {
                    form.advance();
                    return Vec::new();
                }
                if form.fields[3].value() != form.fields[0].value() {
                    self.create.notice =
                        Some(Outside::detail("Type the exact new key name to confirm."));
                    return Vec::new();
                }
                let ttl = if form.fields[2].value().is_empty() {
                    None
                } else {
                    match parse_duration(form.fields[2].value()) {
                        Ok(ttl) => Some(ttl),
                        Err(error) => {
                            self.create.notice = Some(Outside::error(&error));
                            return Vec::new();
                        }
                    }
                };
                self.create.request += 1;
                let effect = CreateEffect::Prepare {
                    id: self.create.request,
                    name: form.fields[0].value().to_string(),
                    words: form
                        .first
                        .then(|| form.fields[1].value().parse().expect("word-count field")),
                    ttl,
                };
                self.create.stage = CreateStage::Preparing;
                self.password = PasswordState::Preparing;
                self.invalidate_reads();
                return vec![Effect::Create(effect)];
            }
            Edit::Rejected(reason) => self.create.notice = Some(Outside::detail(reason)),
            _ => {}
        }
        Vec::new()
    }
}

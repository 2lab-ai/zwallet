//! Tron uses the common send stages, but retains its own reviewed intent and expiration.
use super::*;
use crate::chains::tron as chain;
use crate::wallet::tron_send::{
    self, SendTronTransferOutcome, TronAsset, TronCheckedPayee, TronTransferInput,
    TronTransferReview,
};

#[derive(Default)]
pub struct TronTransfer {
    pub payees: Vec<TronCheckedPayee>,
    pub draft: Option<TronTransferInput>,
    pub review: Option<TronTransferReview>,
}
#[derive(Debug)]
pub enum TronMsg {
    Payees(Result<Vec<TronCheckedPayee>>),
    Review(Result<Box<TronTransferReview>>),
    Changed(Box<TronTransferReview>),
    Progress {
        hash: String,
        expiration: u64,
        journal: PathBuf,
    },
}
pub enum TronJob {
    Review {
        net: Net,
        checked: CheckedList,
        input: Box<TronTransferInput>,
    },
    Send {
        review: Box<TronTransferReview>,
        net: Net,
        nickname: String,
    },
}
impl TronJob {
    pub async fn run(self, id: u64, home: &Path, sender: &UnboundedSender<Msg>) -> TransferMsg {
        match self {
            Self::Review {
                net,
                checked,
                input,
            } => TransferMsg::Tron(TronMsg::Review(
                tron_send::review_tron_transfer(home, &net, &checked, *input)
                    .await
                    .map(Box::new),
            )),
            Self::Send {
                review,
                net,
                nickname,
            } => {
                let journal = chains::SendJournal::new(home, net.chain, review.from())
                    .path()
                    .to_path_buf();
                let tx = sender.clone();
                let progress = Arc::new(move |line: &str| {
                    // The exact flat JSON written by SendRecord follows its last `: `.
                    // Its expiration comes from the fresh confirmed transaction, not the old review.
                    let Some(entry) = line
                        .rsplit_once(": ")
                        .and_then(|(_, s)| serde_json::from_str::<Value>(s).ok())
                    else {
                        return;
                    };
                    let Some(hash) = entry["txHash"]
                        .as_str()
                        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    else {
                        return;
                    };
                    let Some(expiration) = entry["expiresAt"]
                        .as_str()
                        .and_then(chain::send::parse_expiration)
                    else {
                        return;
                    };
                    let _ = tx.send(Msg::Transfer(
                        id,
                        TransferMsg::Tron(TronMsg::Progress {
                            hash: hash.into(),
                            expiration,
                            journal: journal.clone(),
                        }),
                    ));
                });
                match tron_send::send_tron_transfer(*review, &nickname, Some(progress)).await {
                    Ok(SendTronTransferOutcome::Sent(out)) => {
                        TransferMsg::Sent(Ok(SendTransferOutcome::Sent(out)))
                    }
                    Ok(SendTronTransferOutcome::ReviewChanged(review)) => {
                        TransferMsg::Tron(TronMsg::Changed(review))
                    }
                    Err(e) => TransferMsg::Sent(Err(e)),
                }
            }
        }
    }
}
impl App {
    pub fn on_tron(&self) -> bool {
        matches!(self.chain.family, chains::Family::Tron(_))
    }
    pub(super) fn review_tron_draft(&mut self) -> Vec<Effect> {
        let (key, index, from, checked) = match self.transfer_ready() {
            Ok(v) => v,
            Err(e) => {
                self.transfer_error(e);
                return Vec::new();
            }
        };
        let Some(input) = self.transfer.tron.draft.clone() else {
            return Vec::new();
        };
        if (input.key_name.as_str(), input.index, input.from.as_str())
            != (key.as_str(), index, from.as_str())
        {
            self.transfer_error(WalletError::validation(
                "the password-verified key changed; nothing signed",
            ));
            return Vec::new();
        }
        self.transfer.tron.review = None;
        self.transfer.field = None;
        self.transfer.notice = None;
        self.transfer.stage = Stage::Loading;
        self.transfer_job(
            TransferJob::Tron(TronJob::Review {
                net: self.net().clone(),
                checked,
                input: Box::new(input),
            }),
            false,
        )
    }
    pub(super) fn tron_transfer_message(&mut self, event: TronMsg) -> Vec<Effect> {
        match event {
            TronMsg::Payees(Ok(rows)) => {
                self.transfer.tron.payees = rows;
                self.transfer.active = None;
                self.transfer.selected = 0;
                self.transfer.stage = Stage::Payees;
            }
            TronMsg::Review(Ok(review)) => {
                self.transfer.tron.review = Some(*review);
                self.transfer.active = None;
                self.transfer.stage = Stage::Review;
            }
            TronMsg::Changed(review) => {
                self.transfer.tron.review = Some(*review);
                self.transfer.active = None;
                self.transfer.stage = Stage::Review;
                self.transfer.field = None;
                self.transfer.notice = Some(Outside::detail(
                    "review changed; nothing signed; confirm the new values",
                ));
            }
            TronMsg::Payees(Err(e)) | TronMsg::Review(Err(e)) => self.transfer_error(e),
            TronMsg::Progress {
                hash,
                expiration,
                journal,
            } => {
                self.transfer.status = Some(SendStatus {
                    hash,
                    nonce: None,
                    expiration: Some(expiration),
                    journal: Outside::home(&journal.to_string_lossy()),
                    state: ReceiptState::Pending,
                    wait: SEND_WAIT,
                    note: None,
                    task_warning: None,
                    uncertain: false,
                    net: self.net().clone(),
                })
            }
        }
        Vec::new()
    }
    pub(super) fn tron_transfer_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        match (self.transfer.stage, key) {
            (Stage::Payees, Key::Enter) => {
                let Some(payee) = self
                    .transfer
                    .tron
                    .payees
                    .get(self.transfer.selected)
                    .cloned()
                else {
                    return Some(Vec::new());
                };
                let (key_name, index, from, _) = match self.transfer_ready() {
                    Ok(v) => v,
                    Err(e) => {
                        self.transfer_error(e);
                        return Some(Vec::new());
                    }
                };
                let asset = match self.transfer.asset.as_ref() {
                    Some(a) if a.token.is_none() => TronAsset::Trx,
                    Some(a)
                        if a.symbol == "USDT"
                            && a.token.as_deref() == Some(chain::registry::TOKENS[0].address) =>
                    {
                        TronAsset::Usdt
                    }
                    _ => {
                        self.transfer_error(WalletError::validation(
                            "only TRX and registry USDT can be sent on Tron",
                        ));
                        return Some(Vec::new());
                    }
                };
                self.transfer.tron.draft = Some(TronTransferInput {
                    task_id: None,
                    key_name,
                    index,
                    from,
                    payee,
                    asset,
                    amount: String::new(),
                    allow_duplicate: false,
                });
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Amount;
            }
            (Stage::Review, Key::Enter) => {
                if let Some(error) = self
                    .transfer
                    .tron
                    .review
                    .as_ref()
                    .and_then(|r| r.verdict().as_ref().err())
                    .cloned()
                {
                    self.transfer_error(error);
                    return Some(Vec::new());
                }
                if let Err(e) = self.transfer_ready() {
                    self.transfer_error(e);
                    return Some(Vec::new());
                }
                self.transfer.field = Some(Field::new(Kind::Nickname));
                self.transfer.stage = Stage::Confirm;
            }
            (Stage::Review | Stage::Failed, Key::Char('d')) => {
                if let Some(d) = &mut self.transfer.tron.draft {
                    d.allow_duplicate = !d.allow_duplicate;
                }
                return Some(self.review_tron_draft());
            }
            (Stage::Review | Stage::Failed, Key::Char('p') | Key::Char('g')) => {
                self.transfer.notice = Some(Outside::detail(
                    "Tron has no pending nonce or gas-price override; use whitelist fee-cap tron for its energy cap",
                ));
            }
            (Stage::Review | Stage::Failed, Key::Char('r'))
                if self.transfer.tron.draft.is_some() =>
            {
                return Some(self.review_tron_draft());
            }
            (Stage::Review, Key::Esc) => {
                self.transfer.tron.review = None;
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Amount;
            }
            // Once mined, `n` starts a new payment as on EVM (its review runs the repeat check).
            // Tron has no nonce, so nothing replaces the broadcast one.
            (Stage::Status, Key::Char('n')) if self.new_payment_offered() => return None,
            (Stage::Status, Key::Char('n') | Key::Char('s') | Key::Char('x') | Key::Char('R')) => {
                self.transfer.notice = Some(Outside::detail(chain::lookup::SECOND_PAYMENT));
            }
            _ => return None,
        }
        Some(Vec::new())
    }
    pub(super) fn tron_transfer_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        let Some(mut field) = self.transfer.field.take() else {
            return Vec::new();
        };
        self.transfer.notice = None;
        match field.edit(input) {
            Edit::Submit => match self.transfer.stage {
                Stage::Amount => {
                    if let Some(d) = &mut self.transfer.tron.draft {
                        d.amount = field.value().into();
                    }
                    return self.review_tron_draft();
                }
                Stage::Confirm => {
                    let (key, index, from, _) = match self.transfer_ready() {
                        Ok(v) => v,
                        Err(e) => {
                            self.transfer_error(e);
                            return Vec::new();
                        }
                    };
                    if let Some(review) = self.transfer.tron.review.as_ref() {
                        if (review.key_name(), review.index(), review.from())
                            != (key.as_str(), index, from.as_str())
                        {
                            self.transfer_error(WalletError::validation(
                                "the password-verified key changed; nothing signed",
                            ));
                            return Vec::new();
                        }
                        if review.payee().nickname() == field.value() {
                            let review = self.transfer.tron.review.take().unwrap();
                            let nickname = field.value().to_string();
                            self.transfer.stage = Stage::Sending;
                            self.transfer.status = None;
                            self.transfer.retained = None;
                            return self.transfer_job(
                                TransferJob::Tron(TronJob::Send {
                                    review: Box::new(review),
                                    net: self.net().clone(),
                                    nickname,
                                }),
                                false,
                            );
                        }
                    }
                    self.transfer.notice = Some(Outside::detail("type the exact payee nickname"));
                }
                _ => {}
            },
            Edit::Cancel => {
                self.transfer.stage = if self.transfer.stage == Stage::Amount {
                    Stage::Payees
                } else {
                    Stage::Review
                };
                return Vec::new();
            }
            Edit::Interrupt => {
                self.quit = true;
                return Vec::new();
            }
            Edit::Rejected(reason) => self.transfer.notice = Some(Outside::detail(reason)),
            _ => {}
        }
        self.transfer.field = Some(field);
        Vec::new()
    }
}

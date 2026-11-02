//! Native uses the common send stages, but retains its own reviewed intent and expiration.
use super::*;

use crate::wallet::ed25519_send::{
    self, NativeCheckedPayee, NativeSendOutcome, NativeTransferInput, NativeTransferReview,
};

#[derive(Default)]
pub struct NativeTransfer {
    pub payees: Vec<NativeCheckedPayee>,
    pub draft: Option<NativeTransferInput>,
    pub review: Option<NativeTransferReview>,
}
#[derive(Debug)]
pub enum NativeMsg {
    Payees(Result<Vec<NativeCheckedPayee>>),
    Review(Result<Box<NativeTransferReview>>),
    Changed(Box<NativeTransferReview>),
    Progress {
        hash: String,
        nonce: Option<u64>,
        journal: PathBuf,
    },
}
pub enum NativeJob {
    Review {
        net: Net,
        checked: CheckedList,
        input: Box<NativeTransferInput>,
    },
    Send {
        review: Box<NativeTransferReview>,
        net: Net,
        nickname: String,
    },
}
impl NativeJob {
    pub async fn run(self, id: u64, home: &Path, sender: &UnboundedSender<Msg>) -> TransferMsg {
        match self {
            Self::Review {
                net,
                checked,
                input,
            } => TransferMsg::Native(NativeMsg::Review(
                ed25519_send::review_native_transfer(home, &net, &checked, *input)
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
                    let Some(entry) = line
                        .rsplit_once(": ")
                        .and_then(|(_, s)| serde_json::from_str::<Value>(s).ok())
                    else {
                        return;
                    };
                    let Some(hash) = entry["txHash"]
                        .as_str()
                        .filter(|s| super::super::history::parse_hash(net.chain, s).is_ok())
                    else {
                        return;
                    };
                    let nonce = entry["nonce"].as_u64();
                    let _ = tx.send(Msg::Transfer(
                        id,
                        TransferMsg::Native(NativeMsg::Progress {
                            hash: hash.into(),
                            nonce,
                            journal: journal.clone(),
                        }),
                    ));
                });
                match ed25519_send::send_native_transfer(*review, &nickname, Some(progress)).await {
                    Ok(NativeSendOutcome::Sent(out)) => {
                        TransferMsg::Sent(Ok(SendTransferOutcome::Sent(out)))
                    }
                    Ok(NativeSendOutcome::ReviewChanged(review)) => {
                        TransferMsg::Native(NativeMsg::Changed(review))
                    }
                    Err(e) => TransferMsg::Sent(Err(e)),
                }
            }
        }
    }
}
impl App {
    pub fn on_native_ed25519(&self) -> bool {
        matches!(
            self.chain.family,
            chains::Family::Solana(_) | chains::Family::Ton(_)
        )
    }
    pub(super) fn review_native_draft(&mut self) -> Vec<Effect> {
        let (key, index, from, checked) = match self.transfer_ready() {
            Ok(v) => v,
            Err(e) => {
                self.transfer_error(e);
                return Vec::new();
            }
        };
        let Some(input) = self.transfer.native.draft.clone() else {
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
        self.transfer.native.review = None;
        self.transfer.field = None;
        self.transfer.notice = None;
        self.transfer.stage = Stage::Loading;
        self.transfer_job(
            TransferJob::Native(NativeJob::Review {
                net: self.net().clone(),
                checked,
                input: Box::new(input),
            }),
            false,
        )
    }
    pub(super) fn native_transfer_message(&mut self, event: NativeMsg) -> Vec<Effect> {
        match event {
            NativeMsg::Payees(Ok(rows)) => {
                self.transfer.native.payees = rows;
                self.transfer.active = None;
                self.transfer.selected = 0;
                self.transfer.stage = Stage::Payees;
            }
            NativeMsg::Review(Ok(review)) => {
                self.transfer.native.review = Some(*review);
                self.transfer.active = None;
                self.transfer.stage = Stage::Review;
            }
            NativeMsg::Changed(review) => {
                self.transfer.native.review = Some(*review);
                self.transfer.active = None;
                self.transfer.stage = Stage::Review;
                self.transfer.field = None;
                self.transfer.notice = Some(Outside::detail(
                    "review changed; nothing signed; confirm the new values",
                ));
            }
            NativeMsg::Payees(Err(e)) | NativeMsg::Review(Err(e)) => self.transfer_error(e),
            NativeMsg::Progress {
                hash,
                nonce,
                journal,
            } => {
                self.transfer.status = Some(SendStatus {
                    hash,
                    nonce,
                    expiration: None,
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
    pub(super) fn native_transfer_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        match (self.transfer.stage, key) {
            (Stage::Payees, Key::Enter) => {
                let Some(payee) = self
                    .transfer
                    .native
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
                if self
                    .transfer
                    .asset
                    .as_ref()
                    .is_none_or(|a| a.token.is_some())
                {
                    self.transfer_error(WalletError::validation(
                        "only the native coin is supported on this family",
                    ));
                    return Some(Vec::new());
                }
                self.transfer.native.draft = Some(NativeTransferInput {
                    task_id: None,
                    key_name,
                    index,
                    from,
                    payee,
                    amount: String::new(),
                    allow_duplicate: false,
                });
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Amount;
            }
            (Stage::Review, Key::Enter) => {
                if self
                    .transfer
                    .native
                    .review
                    .as_ref()
                    .is_some_and(|r| !r.sufficient_funds())
                {
                    self.transfer.notice = Some(Outside::detail(
                        "Insufficient funds; confirmation is refused",
                    ));
                    return Some(Vec::new());
                }
                if let Some(error) = self
                    .transfer
                    .native
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
                if let Some(d) = &mut self.transfer.native.draft {
                    d.allow_duplicate = !d.allow_duplicate;
                }
                return Some(self.review_native_draft());
            }
            (Stage::Review | Stage::Failed, Key::Char('p') | Key::Char('g')) => {
                self.transfer.notice = Some(Outside::detail(
                    "No gas-price or pending override on this family; edit its signed fee cap",
                ));
            }
            (Stage::Review | Stage::Failed, Key::Char('r'))
                if self.transfer.native.draft.is_some() =>
            {
                return Some(self.review_native_draft());
            }
            (Stage::Review, Key::Esc) => {
                self.transfer.native.review = None;
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Amount;
            }
            // Once mined, `n` starts a new payment as on EVM (its review runs the repeat check).
            // These native families do not expose transaction replacement.
            (Stage::Status, Key::Char('n')) if self.new_payment_offered() => return None,
            (Stage::Status, Key::Char('n') | Key::Char('s') | Key::Char('x') | Key::Char('R')) => {
                self.transfer.notice = Some(Outside::detail(
                    "No replacement on this family. Sending again may pay twice; inspect the existing transaction first.",
                ));
            }
            _ => return None,
        }
        Some(Vec::new())
    }
    pub(super) fn native_transfer_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        let Some(mut field) = self.transfer.field.take() else {
            return Vec::new();
        };
        self.transfer.notice = None;
        match field.edit(input) {
            Edit::Submit => match self.transfer.stage {
                Stage::Amount => {
                    if let Some(d) = &mut self.transfer.native.draft {
                        d.amount = field.value().into();
                    }
                    return self.review_native_draft();
                }
                Stage::Confirm => {
                    let (key, index, from, _) = match self.transfer_ready() {
                        Ok(v) => v,
                        Err(e) => {
                            self.transfer_error(e);
                            return Vec::new();
                        }
                    };
                    if let Some(review) = self.transfer.native.review.as_ref() {
                        if (review.key_name(), review.index(), review.from())
                            != (key.as_str(), index, from.as_str())
                        {
                            self.transfer_error(WalletError::validation(
                                "the password-verified key changed; nothing signed",
                            ));
                            return Vec::new();
                        }
                        if review.payee().nickname() == field.value() {
                            let review = self.transfer.native.review.take().unwrap();
                            let nickname = field.value().to_string();
                            self.transfer.stage = Stage::Sending;
                            self.transfer.status = None;
                            self.transfer.retained = None;
                            return self.transfer_job(
                                TransferJob::Native(NativeJob::Send {
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

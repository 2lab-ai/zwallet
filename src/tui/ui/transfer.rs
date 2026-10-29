//! Every displayed amount and bound comes from the immutable decoded review.
use super::*;
use crate::chains::evm::tx::Action;
use crate::tui::app::transfer::{ReceiptState, Stage};
use alloy::primitives::U256;

pub(super) fn pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let block = Block::bordered().border_set(ASCII).title(" send ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let state = app.transfer();
    if app.on_tron()
        && matches!(
            state.stage,
            Stage::Payees | Stage::Amount | Stage::Review | Stage::Confirm
        )
    {
        super::tron_transfer::draw(frame, inner, app, placed);
        return;
    }
    let row = |frame: &mut Frame,
               y: u16,
               label: &str,
               value: Option<Outside>,
               style: Style,
               placed: &mut Vec<Placed>| {
        let r = Row::default().text(label, style);
        let r = if let Some(value) = value {
            r.outside(&value, style)
        } else {
            r
        };
        r.draw(frame, line(inner, y), placed);
    };
    match state.stage {
        Stage::Inactive | Stage::Loading => {
            row(frame, 0, "reading checked send data...", None, HINT, placed)
        }
        Stage::Assets => {
            row(
                frame,
                0,
                "Select an asset; only registry tokens with a balance are listed.",
                None,
                HINT,
                placed,
            );
            let room = usize::from(inner.height.saturating_sub(1));
            let offset = state.selected.saturating_sub(room.saturating_sub(1));
            for (i, asset) in state.assets.iter().enumerate().skip(offset).take(room) {
                row(
                    frame,
                    (i - offset + 1) as u16,
                    "",
                    Some(Outside::detail(&App::asset_label(asset))),
                    if i == state.selected {
                        SELECTED
                    } else {
                        NORMAL
                    },
                    placed,
                );
            }
        }
        Stage::Payees => {
            row(
                frame,
                0,
                "Pick a password-checked payee. Raw addresses are not accepted.",
                None,
                HINT,
                placed,
            );
            if state.payees.is_empty() {
                row(
                    frame,
                    2,
                    "No address entries on this chain; use zwallet whitelist add.",
                    None,
                    WARNING,
                    placed,
                );
            }
            let room = usize::from(inner.height.saturating_sub(1)) / 2;
            let offset = state.selected.saturating_sub(room.saturating_sub(1));
            for (i, payee) in state.payees.iter().enumerate().skip(offset).take(room) {
                let style = if i == state.selected {
                    SELECTED
                } else {
                    NORMAL
                };
                let y = (1 + (i - offset) * 2) as u16;
                row(
                    frame,
                    y,
                    "",
                    Some(Outside::nickname(payee.nickname())),
                    style,
                    placed,
                );
                row(
                    frame,
                    y + 1,
                    "",
                    Some(Outside::address(payee.address())),
                    style,
                    placed,
                );
            }
        }
        Stage::Amount => {
            if let Some(draft) = &state.draft {
                row(
                    frame,
                    0,
                    "Payee: ",
                    Some(Outside::nickname(draft.payee.nickname())),
                    TITLE,
                    placed,
                );
                row(
                    frame,
                    1,
                    "To: ",
                    Some(Outside::address(draft.payee.address())),
                    NORMAL,
                    placed,
                );
            }
            if let Some(asset) = &state.asset {
                row(
                    frame,
                    3,
                    "",
                    Some(Outside::detail(&App::asset_label(asset))),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    4,
                    "",
                    Some(Outside::detail(&format!(
                        "{} decimals; enter the amount in {}",
                        asset.decimals, asset.symbol
                    ))),
                    HINT,
                    placed,
                );
            }
            field(frame, inner, 6, "Amount: ", app, placed);
        }
        Stage::Review | Stage::Confirm | Stage::Fee => {
            let Some(review) = &state.review else {
                row(
                    frame,
                    0,
                    "Set a new fee ceiling to prepare the review again.",
                    None,
                    HINT,
                    placed,
                );
                field(frame, inner, 2, "Gas ceiling (gwei): ", app, placed);
                return;
            };
            row(
                frame,
                0,
                "Payee: ",
                Some(Outside::nickname(review.payee().nickname())),
                TITLE,
                placed,
            );
            row(
                frame,
                1,
                "To: ",
                Some(Outside::address(review.payee().address())),
                NORMAL,
                placed,
            );
            row(
                frame,
                2,
                "From: ",
                Some(Outside::address(review.from())),
                NORMAL,
                placed,
            );
            let asset = state.asset.as_ref();
            let (amount, token) = match review.intent().action() {
                Action::NativeTransfer { value, .. } => (*value, None),
                Action::TransferCall { amount, token, .. } => {
                    (*amount, Some(token.to_checksum(None)))
                }
                _ => (U256::ZERO, None),
            };
            let formatted =
                crate::core::amount::format_units(amount, asset.map_or(0, |a| a.decimals));
            let label = format!(
                "{} {} / {} decimals / chainId {}",
                formatted,
                asset.map_or("?", |a| a.symbol.as_str()),
                asset.map_or(0, |a| a.decimals),
                review.intent().chain_id()
            );
            let label = if label.len() <= 72 {
                label
            } else {
                format!(
                    "{} / {} decimals / chainId {}; exact amount below",
                    asset.map_or("?", |a| a.symbol.as_str()),
                    asset.map_or(0, |a| a.decimals),
                    review.intent().chain_id()
                )
            };
            row(frame, 3, "", Some(Outside::detail(&label)), NORMAL, placed);
            row(
                frame,
                4,
                "",
                Some(Outside::detail(&format!(
                    "Base-unit amount; gas cap wei: {}",
                    review
                        .options()
                        .max_fee_wei
                        .map_or_else(|| "auto".into(), |fee| fee.to_string())
                ))),
                HINT,
                placed,
            );
            row(
                frame,
                5,
                "",
                Some(Outside::quantity(&amount.to_string())),
                NORMAL,
                placed,
            );
            row(
                frame,
                6,
                "",
                Some(Outside::detail(&format!(
                    "Nonce: {} / calldata: {}",
                    review.intent().nonce(),
                    if token.is_some() {
                        "ERC20 transfer 0xa9059cbb"
                    } else {
                        "0x (native)"
                    }
                ))),
                NORMAL,
                placed,
            );
            if let Some(token) = token {
                row(
                    frame,
                    7,
                    "Token: ",
                    Some(Outside::address(&token)),
                    NORMAL,
                    placed,
                );
            }
            row(
                frame,
                8,
                "Max gas wei: ",
                Some(Outside::quantity(&review.max_gas_cost().to_string())),
                NORMAL,
                placed,
            );
            row(
                frame,
                9,
                "L1 fee estimate (wei; can rise after confirm):",
                None,
                HINT,
                placed,
            );
            row(
                frame,
                10,
                "",
                Some(Outside::quantity(
                    &review.l1_fee().unwrap_or_default().to_string(),
                )),
                NORMAL,
                placed,
            );
            row(
                frame,
                11,
                "Maximum total estimate (wei):",
                None,
                HINT,
                placed,
            );
            row(
                frame,
                12,
                "",
                Some(Outside::quantity(&review.max_total_fee().to_string())),
                TITLE,
                placed,
            );
            row(
                frame,
                13,
                "Checked policy execution-fee cap (wei):",
                None,
                HINT,
                placed,
            );
            row(
                frame,
                14,
                "",
                Some(Outside::quantity(
                    &review.fee_cap_wei().unwrap_or_default().to_string(),
                )),
                NORMAL,
                placed,
            );
            row(
                frame,
                15,
                if review.verdict().is_ok() {
                    "Whitelist: allowed"
                } else {
                    "Whitelist: DENIED (Enter shows reason)"
                },
                None,
                if review.verdict().is_ok() {
                    NORMAL
                } else {
                    ERROR
                },
                placed,
            );
            let pending = review
                .fields()
                .get("pendingCount")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let repeats = review
                .fields()
                .get("duplicateSends")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            row(
                frame,
                16,
                "",
                Some(Outside::detail(&format!(
                    "Funds {} | pending {} {} | repeats {} {}",
                    if review.sufficient_funds() {
                        "yes"
                    } else {
                        "NO"
                    },
                    pending,
                    if review.options().allow_pending {
                        "OVERRIDE"
                    } else {
                        "blocked"
                    },
                    repeats,
                    if review.options().allow_duplicate {
                        "OVERRIDE"
                    } else {
                        "blocked"
                    }
                ))),
                if pending > 0 || repeats > 0 || !review.sufficient_funds() {
                    WARNING
                } else {
                    NORMAL
                },
                placed,
            );
            if state.stage == Stage::Confirm {
                field(frame, inner, 17, "Type payee nickname: ", app, placed);
            } else if state.stage == Stage::Fee {
                field(frame, inner, 17, "Gas ceiling (gwei): ", app, placed);
            } else if review.options().allow_pending || review.options().allow_duplicate {
                row(
                    frame,
                    17,
                    "Override: a new send is a new payment and may pay twice.",
                    None,
                    WARNING,
                    placed,
                );
            } else {
                row(
                    frame,
                    17,
                    "Enter reviews the typed confirmation; no signature yet.",
                    None,
                    HINT,
                    placed,
                );
            }
        }
        Stage::Sending | Stage::Status | Stage::Wait => {
            row(
                frame,
                0,
                if app.sending() {
                    "Signing / broadcasting; q waits for the result."
                } else {
                    "Send recorded; leaving this screen does not cancel it."
                },
                None,
                TITLE,
                placed,
            );
            if let Some(status) = &state.status {
                row(
                    frame,
                    2,
                    "Hash: ",
                    Some(Outside::transaction_hash(&status.hash)),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    3,
                    "",
                    Some(Outside::detail(&match status.nonce {
                        Some(n) => format!("Nonce: {n}"),
                        None => format!(
                            "Expires: {}",
                            status
                                .expiration
                                .map(crate::chains::tron::send::expires_at)
                                .unwrap_or_else(|| "unknown".into())
                        ),
                    })),
                    NORMAL,
                    placed,
                );
                row(frame, 4, "Journal:", None, HINT, placed);
                row(frame, 5, "", Some(status.journal.clone()), NORMAL, placed);
                row(
                    frame,
                    7,
                    match status.state {
                        ReceiptState::Pending => "pending; checking the receipt every 3 seconds",
                        ReceiptState::Success => "mined: success",
                        ReceiptState::Reverted if app.on_tron() => {
                            "mined: failed (fees may have burned)"
                        }
                        ReceiptState::Reverted => "mined: reverted",
                        ReceiptState::MinedUnknown => "mined: execution result unknown",
                        ReceiptState::ExpiredUnseen => {
                            "expired-unseen at this node; another node may have it"
                        }
                        ReceiptState::Unknown => "unknown - run zwallet tx; do not resend blindly",
                    },
                    None,
                    if status.state == ReceiptState::Success {
                        NORMAL
                    } else {
                        WARNING
                    },
                    placed,
                );
                row(
                    frame,
                    8,
                    "",
                    Some(Outside::detail(&format!(
                        "Polling deadline: {} seconds from the latest wait",
                        status.wait.as_secs()
                    ))),
                    HINT,
                    placed,
                );
                if app.on_tron() {
                    row(
                        frame,
                        10,
                        "Tron has no nonce replacement.",
                        None,
                        HINT,
                        placed,
                    );
                } else {
                    row(frame, 10, app.replacement_hint(), None, HINT, placed);
                    for (offset, chunk) in app
                        .replacement_status_notes()
                        .into_iter()
                        .flat_map(|note| note.as_bytes().chunks(72))
                        .enumerate()
                    {
                        row(
                            frame,
                            11 + offset as u16,
                            "",
                            Some(Outside::detail(&String::from_utf8_lossy(chunk))),
                            WARNING,
                            placed,
                        );
                    }
                }
                if let Some(note) = &status.note {
                    if app.on_tron() {
                        let chars: Vec<_> = note.text().chars().collect();
                        let rows = if state.stage == Stage::Wait { 3 } else { 6 };
                        for (i, chunk) in chars
                            .chunks(usize::from(inner.width).max(1))
                            .take(rows)
                            .enumerate()
                        {
                            row(
                                frame,
                                12 + i as u16,
                                "",
                                Some(Outside::warning(&chunk.iter().collect::<String>())),
                                WARNING,
                                placed,
                            );
                        }
                    } else if state.stage != Stage::Wait {
                        row(
                            frame,
                            17,
                            "",
                            Some(note.cut(usize::from(inner.width))),
                            ERROR,
                            placed,
                        );
                    }
                }
            }
            if state.stage == Stage::Wait {
                field(frame, inner, 17, "Wait from now (e.g. 5m): ", app, placed);
            }
        }
        Stage::Failed => {
            if let Some(error) = &state.notice {
                error_view(frame, inner, "The send did not complete:", error, placed);
            }
        }
    }
}
fn field(frame: &mut Frame, area: Rect, y: u16, label: &str, app: &App, placed: &mut Vec<Placed>) {
    if let Some(field) = &app.transfer().field {
        Row::default()
            .text(label, HINT)
            .outside(&Outside::input(&field.drawn()), NORMAL)
            .draw(frame, line(area, y), placed);
    }
}

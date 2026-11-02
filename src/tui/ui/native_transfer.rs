//! Native SOL/TON review. Every value comes from the immutable wallet review.
use super::*;
use crate::core::amount::format_units;
use crate::tui::app::transfer::Stage;

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let state = app.transfer();
    let row =
        |frame: &mut Frame, y, label: &str, value: Outside, style, placed: &mut Vec<Placed>| {
            Row::default()
                .text(label, style)
                .outside(&value, style)
                .draw(frame, line(area, y), placed);
        };
    match state.stage {
        Stage::Payees => {
            row(
                frame,
                0,
                "",
                Outside::detail("Choose a password-checked payee scoped to this chain."),
                HINT,
                placed,
            );
            if state.native.payees.is_empty() {
                row(
                    frame,
                    2,
                    "",
                    Outside::detail("No payees. Add one in 7 whitelist for this chain."),
                    WARNING,
                    placed,
                );
            }
            let room = usize::from(area.height.saturating_sub(1)) / 2;
            let offset = state.selected.saturating_sub(room.saturating_sub(1));
            for (i, payee) in state
                .native
                .payees
                .iter()
                .enumerate()
                .skip(offset)
                .take(room)
            {
                let style = if i == state.selected {
                    SELECTED
                } else {
                    NORMAL
                };
                row(
                    frame,
                    1 + 2 * (i - offset) as u16,
                    "",
                    Outside::nickname(payee.nickname()),
                    style,
                    placed,
                );
                row(
                    frame,
                    2 + 2 * (i - offset) as u16,
                    "",
                    Outside::address(payee.address()),
                    style,
                    placed,
                );
            }
        }
        Stage::Amount => {
            if let Some(draft) = &state.native.draft {
                row(
                    frame,
                    0,
                    "Payee: ",
                    Outside::nickname(draft.payee.nickname()),
                    TITLE,
                    placed,
                );
                row(
                    frame,
                    1,
                    "To: ",
                    Outside::address(draft.payee.address()),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    2,
                    "From: ",
                    Outside::address(&draft.from),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    4,
                    "",
                    Outside::detail(&format!(
                        "{}; {} decimals; greater than zero",
                        app.chain().symbol,
                        app.chain().decimals
                    )),
                    HINT,
                    placed,
                );
            }
            field(frame, area, 6, "Amount: ", app, placed);
        }
        Stage::Review | Stage::Confirm => {
            let Some(review) = &state.native.review else {
                return;
            };
            let fields = review.fields();
            row(
                frame,
                0,
                "Payee: ",
                Outside::nickname(review.payee().nickname()),
                TITLE,
                placed,
            );
            row(
                frame,
                1,
                "To: ",
                Outside::address(review.payee().address()),
                NORMAL,
                placed,
            );
            row(
                frame,
                2,
                "From: ",
                Outside::address(review.from()),
                NORMAL,
                placed,
            );
            row(
                frame,
                3,
                "Amount: ",
                Outside::detail(&format!(
                    "{} {}",
                    fields["amount"].as_str().unwrap_or("?"),
                    app.chain().symbol
                )),
                TITLE,
                placed,
            );
            row(
                frame,
                4,
                "Exact base units: ",
                Outside::quantity(fields["amountRaw"].as_str().unwrap_or("?")),
                NORMAL,
                placed,
            );
            let ton = matches!(app.chain().family, crate::chains::Family::Ton(_));
            row(
                frame,
                5,
                if ton { "RPC fee estimate: " } else { "Fee: " },
                Outside::detail(&format!(
                    "{} {}",
                    format_units(review.max_fee(), app.chain().decimals),
                    app.chain().symbol
                )),
                NORMAL,
                placed,
            );
            row(
                frame,
                6,
                "Signed policy fee cap: ",
                Outside::detail(
                    &review
                        .fee_cap()
                        .map(|v| format_units(v, app.chain().decimals))
                        .unwrap_or_else(|| "missing".into()),
                ),
                NORMAL,
                placed,
            );
            if ton {
                row(
                    frame,
                    7,
                    "",
                    Outside::detail("TON fees are not capped by the signed wallet message."),
                    WARNING,
                    placed,
                );
            } else {
                row(
                    frame,
                    7,
                    "",
                    Outside::detail("One SystemProgram transfer; no other instructions."),
                    HINT,
                    placed,
                );
            }
            let verdict = match review.verdict() {
                Ok(()) if review.sufficient_funds() => {
                    "Whitelist allowed; balance sufficient".into()
                }
                Ok(()) => "Insufficient funds; confirmation is refused".into(),
                Err(e) => format!("{}: {}", e.code.as_str(), e.message),
            };
            row(
                frame,
                8,
                "",
                Outside::detail(&verdict),
                if review.verdict().is_ok() && review.sufficient_funds() {
                    NORMAL
                } else {
                    WARNING
                },
                placed,
            );
            let repeats = fields["duplicateSends"].as_array().map_or(0, Vec::len);
            let window = fields["duplicateGuard"]["windowSecs"]
                .as_u64()
                .unwrap_or(600);
            let override_on = state
                .native
                .draft
                .as_ref()
                .is_some_and(|d| d.allow_duplicate);
            row(
                frame,
                9,
                "",
                Outside::detail(&format!(
                    "Repeats {repeats}; {} [{window}s/1MiB local]",
                    if override_on {
                        "OVERRIDE"
                    } else {
                        "no override"
                    }
                )),
                if override_on || repeats > 0 {
                    WARNING
                } else {
                    HINT
                },
                placed,
            );
            row(
                frame,
                10,
                "",
                Outside::detail("A new signature/send may pay twice; no replacement support."),
                WARNING,
                placed,
            );
            if let Some(expiry) = fields.get("validUntil").and_then(serde_json::Value::as_u64) {
                row(
                    frame,
                    11,
                    "",
                    Outside::detail(&format!(
                        "V4R2 seqno {}; expires at Unix {expiry}",
                        fields["seqno"]
                    )),
                    NORMAL,
                    placed,
                );
            } else if let Some(expiry) = fields.get("expiresAt").and_then(serde_json::Value::as_str)
            {
                row(
                    frame,
                    11,
                    "Expires: ",
                    Outside::detail(expiry),
                    NORMAL,
                    placed,
                );
            } else if let Some(height) = fields.get("lastValidBlockHeight") {
                row(
                    frame,
                    11,
                    "Last valid block height: ",
                    Outside::detail(&height.to_string()),
                    NORMAL,
                    placed,
                );
            }
            if let Some(warning) = fields.get("warning").and_then(serde_json::Value::as_str) {
                let warning = Outside::warning(warning);
                for (i, chunk) in warning
                    .text()
                    .as_bytes()
                    .chunks(usize::from(area.width))
                    .take(4)
                    .enumerate()
                {
                    row(
                        frame,
                        12 + i as u16,
                        "",
                        Outside::warning(std::str::from_utf8(chunk).expect("ASCII warning")),
                        WARNING,
                        placed,
                    );
                }
            }
            if state.stage == Stage::Confirm {
                field(frame, area, 17, "Type payee nickname: ", app, placed);
            } else {
                row(
                    frame,
                    17,
                    "",
                    Outside::detail("Enter opens confirmation; no signature yet."),
                    HINT,
                    placed,
                );
            }
        }
        _ => {}
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

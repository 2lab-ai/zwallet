//! Tron review rows use the held decoded payment and integer cost bounds.
use super::*;
use crate::{core::amount::format_units, tui::app::transfer::Stage};

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
                Outside::detail("Choose a password-checked Tron-only payee."),
                HINT,
                placed,
            );
            if state.tron.payees.is_empty() {
                row(
                    frame,
                    2,
                    "",
                    Outside::detail(
                        "No Tron payees; whitelist add <nick> <T-address> --chains tron",
                    ),
                    WARNING,
                    placed,
                );
            }
            let room = usize::from(area.height.saturating_sub(1)) / 2;
            let offset = state.selected.saturating_sub(room.saturating_sub(1));
            for (i, payee) in state.tron.payees.iter().enumerate().skip(offset).take(room) {
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
            if let Some(draft) = &state.tron.draft {
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
                        "{}; 6 decimals; greater than zero",
                        draft.asset.symbol()
                    )),
                    HINT,
                    placed,
                );
            }
            field(frame, area, 6, "Amount: ", app, placed);
        }
        Stage::Review | Stage::Confirm => {
            let Some(review) = &state.tron.review else {
                return;
            };
            let fields = review.fields();
            let fee = &fields["fees"];
            let value = |key: &str| {
                fee.get(key)
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
                    .unwrap_or_else(|| "?".into())
            };
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
            let amount = format!(
                "{} {} (6 decimals; exact base units below)",
                fields["amount"].as_str().unwrap_or("?"),
                review.asset().symbol()
            );
            // U256 token amounts can occupy all 78 inner columns. Their exact raw value
            // gets its own unprefixed row; never cut away its asset or decimal scale.
            let amount = if amount.len() <= usize::from(area.width.saturating_sub(8)).min(72) {
                amount
            } else {
                format!(
                    "{}; 6 decimals; exact base units below",
                    review.asset().symbol()
                )
            };
            row(
                frame,
                3,
                "Amount: ",
                Outside::detail(&amount),
                NORMAL,
                placed,
            );
            row(
                frame,
                4,
                "",
                Outside::quantity(fields["amountRaw"].as_str().unwrap_or("?")),
                NORMAL,
                placed,
            );
            if let Some(token) = fields.get("token").and_then(serde_json::Value::as_str) {
                row(frame, 5, "Token: ", Outside::address(token), NORMAL, placed);
            }
            row(
                frame,
                6,
                "Bandwidth: ",
                Outside::detail(&format!(
                    "{} bytes / {} / burn {} sun / activation {} sun",
                    value("bandwidthBytes"),
                    value("bandwidthSource"),
                    value("bandwidthSun"),
                    value("activationSun")
                )),
                NORMAL,
                placed,
            );
            row(
                frame,
                7,
                "Energy: ",
                Outside::detail(&format!(
                    "{} / burn {} sun",
                    value("energy"),
                    value("energySun")
                )),
                NORMAL,
                placed,
            );
            row(
                frame,
                8,
                "feeLimit: ",
                Outside::detail(&format!("{} sun", value("feeLimit"))),
                NORMAL,
                placed,
            );
            row(
                frame,
                9,
                "Estimated cost TRX: ",
                Outside::detail(&format_units(review.estimated_cost_sun(), 6)),
                NORMAL,
                placed,
            );
            row(
                frame,
                10,
                "Maximum cost TRX: ",
                Outside::detail(&format_units(review.max_cost_sun(), 6)),
                TITLE,
                placed,
            );
            row(
                frame,
                11,
                "Maximum fee TRX: ",
                Outside::detail(&format_units(review.max_fee_sun(), 6)),
                NORMAL,
                placed,
            );
            row(
                frame,
                12,
                "TRX left at least: ",
                Outside::detail(&format_units(review.trx_left_sun(), 6)),
                NORMAL,
                placed,
            );
            row(
                frame,
                13,
                "At prices of block: ",
                Outside::detail(&value("pricesAtBlock")),
                HINT,
                placed,
            );
            row(
                frame,
                14,
                "Expires: ",
                Outside::detail(fields["expiresAt"].as_str().unwrap_or("?")),
                NORMAL,
                placed,
            );
            let verdict = match review.verdict() {
                Ok(()) => format!(
                    "Whitelist allowed; energy cap {} TRX",
                    review
                        .fee_cap_sun()
                        .map(|v| format_units(v, 6))
                        .unwrap_or_else(|| "missing".into())
                ),
                Err(e) => format!("{}: {}", e.code.as_str(), e.message),
            };
            row(
                frame,
                15,
                "",
                Outside::detail(&verdict),
                if review.verdict().is_ok() {
                    NORMAL
                } else {
                    WARNING
                },
                placed,
            );
            let repeats = fields["duplicateSends"].as_array().map_or(0, Vec::len);
            row(
                frame,
                16,
                "",
                Outside::detail(&{
                    let window = fields["duplicateGuard"]["windowSecs"]
                        .as_u64()
                        .unwrap_or(600);
                    if review.allow_duplicate() {
                        format!(
                            "Repeat {repeats} OVERRIDE [{window}s/1MiB local]: a new send may pay twice"
                        )
                    } else {
                        format!("Repeats {repeats}; override false; {window}s/1MiB local aid")
                    }
                }),
                if repeats > 0 { WARNING } else { HINT },
                placed,
            );
            if state.stage == Stage::Confirm {
                field(frame, area, 17, "Type payee nickname: ", app, placed);
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

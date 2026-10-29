//! One result source per wrapped block; reviewed messages retain all bytes between brackets.
use super::*;
use crate::core::amount::format_units;
use crate::tui::app::tools::{Purpose, Stage, Tool, ToolResult};

pub(super) fn pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let state = app.tools();
    let block = Block::bordered()
        .border_set(ASCII)
        .title(format!(" tools / {} ", state.tool.title()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let row = |frame: &mut Frame,
               y: u16,
               label: &str,
               value: Option<Outside>,
               style: Style,
               placed: &mut Vec<Placed>| {
        let r = Row::default().text(label, style);
        let r = if let Some(v) = value {
            r.outside(&v, style)
        } else {
            r
        };
        r.draw(frame, line(inner, y), placed);
    };
    match state.stage {
        Stage::Menu => {
            for (i, tool) in Tool::ALL.iter().enumerate() {
                row(
                    frame,
                    (i * 2) as u16,
                    tool.title(),
                    None,
                    if i == state.selected {
                        SELECTED
                    } else {
                        NORMAL
                    },
                    placed,
                );
            }
            row(
                frame,
                12,
                "Calls and token balances are reads. Invokes and messages require warm-up.",
                None,
                HINT,
                placed,
            );
        }
        Stage::Contracts => {
            row(
                frame,
                0,
                "Choose a contract from the password-checked list.",
                None,
                HINT,
                placed,
            );
            if state.contracts.is_empty() {
                row(
                    frame,
                    2,
                    "No checked contracts on this chain. Use 5 whitelist to add one.",
                    None,
                    WARNING,
                    placed,
                );
            }
            let room = usize::from(inner.height.saturating_sub(1)) / 3;
            let offset = state.selected.saturating_sub(room.saturating_sub(1));
            for (i, contract) in state.contracts.iter().enumerate().skip(offset).take(room) {
                let y = (1 + (i - offset) * 3) as u16;
                let style = if i == state.selected {
                    SELECTED
                } else {
                    NORMAL
                };
                row(
                    frame,
                    y,
                    "",
                    Some(Outside::nickname(contract.nickname())),
                    style,
                    placed,
                );
                row(
                    frame,
                    y + 1,
                    "",
                    Some(Outside::address(contract.address())),
                    style,
                    placed,
                );
                row(
                    frame,
                    y + 2,
                    "Selectors: ",
                    Some(
                        Outside::input(&contract.selectors().join(", "))
                            .cut(usize::from(inner.width).saturating_sub(11)),
                    ),
                    HINT,
                    placed,
                );
            }
        }
        Stage::Field => {
            if let Some(contract) = &state.contract {
                row(
                    frame,
                    0,
                    "Contract: ",
                    Some(Outside::nickname(contract.nickname())),
                    TITLE,
                    placed,
                );
                row(
                    frame,
                    1,
                    "",
                    Some(Outside::address(contract.address())),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    2,
                    "Selectors: ",
                    Some(
                        Outside::input(&contract.selectors().join(", "))
                            .cut(usize::from(inner.width).saturating_sub(11)),
                    ),
                    HINT,
                    placed,
                );
            }
            let label = match state.purpose {
                Purpose::Address => "Contract address",
                Purpose::Signature => "Function signature (name(types)(returns))",
                Purpose::Argument(_) => "One argument (exact string or ABI array/tuple)",
                Purpose::Data => "Complete 0x calldata",
                Purpose::Value => "Native value",
                Purpose::Message => "Message: printable ASCII, at most 512 bytes",
                Purpose::Token => "ERC-20 contract address",
                Purpose::Fee => "Maximum gas price (gwei)",
                Purpose::Confirm => "Contract nickname",
                Purpose::SignConfirm => "Type sign exactly",
            };
            row(frame, 4, label, None, TITLE, placed);
            if let Purpose::Argument(i) = state.purpose {
                row(frame, 5, &format!("Argument {}", i + 1), None, HINT, placed);
            }
            field(frame, inner, 7, app, placed);
            if state.purpose == Purpose::Message {
                row(
                    frame,
                    10,
                    "Other text: zwallet sign-message",
                    None,
                    HINT,
                    placed,
                );
                row(
                    frame,
                    11,
                    "Review shows every byte; leading and trailing spaces are preserved.",
                    None,
                    HINT,
                    placed,
                );
            }
        }
        Stage::Loading => row(frame, 0, "Reading and checking...", None, HINT, placed),
        Stage::Signing => row(
            frame,
            0,
            "Signing only the held review bytes...",
            None,
            HINT,
            placed,
        ),
        Stage::MessageReview | Stage::MessageConfirm => {
            let Some(review) = &state.message else {
                return;
            };
            row(
                frame,
                0,
                "Key: ",
                Some(Outside::key_name(&review.key().name)),
                TITLE,
                placed,
            );
            row(
                frame,
                1,
                "Address: ",
                Some(Outside::address(review.address())),
                NORMAL,
                placed,
            );
            row(
                frame,
                2,
                &format!("{} / EIP-191 message", review.chain().key),
                None,
                HINT,
                placed,
            );
            // Only the message occupies these rows. Exactly 76 message bytes per bracketed row.
            for (i, text) in review.rows().iter().enumerate() {
                row(
                    frame,
                    4 + i as u16,
                    "",
                    Some(Outside::input(text)),
                    NORMAL,
                    placed,
                );
            }
            row(frame, 12, &review.metadata(), None, HINT, placed);
            if state.stage == Stage::MessageConfirm {
                row(
                    frame,
                    14,
                    "Type sign exactly to sign the bytes above:",
                    None,
                    TITLE,
                    placed,
                );
                field(frame, inner, 15, app, placed);
            } else {
                row(
                    frame,
                    14,
                    "A message signature can grant spending authority to a contract.",
                    None,
                    WARNING,
                    placed,
                );
            }
        }
        Stage::InvokeReview | Stage::Confirm => {
            let Some(review) = &state.invoke else {
                return;
            };
            let intent = review.intent();
            let mut values = vec![
                (
                    format!(
                        "Contract: {}",
                        Outside::nickname(review.contract().nickname()).text()
                    ),
                    TITLE,
                ),
                (format!("To: {}", intent.to().to_checksum(None)), NORMAL),
                (
                    format!(
                        "From: {}",
                        review.key().address.as_deref().unwrap_or("unknown")
                    ),
                    NORMAL,
                ),
                (
                    format!(
                        "contract-call / chainId {} / nonce {}",
                        intent.chain_id(),
                        intent.nonce()
                    ),
                    NORMAL,
                ),
                (
                    format!(
                        "selector 0x{}",
                        alloy::hex::encode(intent.input().get(..4).unwrap_or_default())
                    ),
                    NORMAL,
                ),
                ("Native value in base units (exact):".into(), HINT),
                (intent.value().to_string(), NORMAL),
                ("Maximum execution fee in wei:".into(), HINT),
                (review.max_gas_cost().to_string(), NORMAL),
                ("Execution fee + estimated L1 fee in wei:".into(), HINT),
                (review.max_total_fee().to_string(), NORMAL),
            ];
            // The send guards' verdicts, as the transfer review shows them: what `p` and `d`
            // would override, before the nickname is asked for.
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
            values.push((
                format!(
                    "pending {pending} {} | repeats {repeats} {}",
                    guard_verdict(pending, state.options.allow_pending),
                    guard_verdict(repeats as u64, state.options.allow_duplicate)
                ),
                if pending > 0 || repeats > 0 {
                    WARNING
                } else {
                    NORMAL
                },
            ));
            if state.options.allow_pending || state.options.allow_duplicate {
                values.push((
                    "Override: a new send is a new payment and may pay twice.".into(),
                    WARNING,
                ));
            }
            match review.verdict() {
                Ok(()) => values.push(("Checked whitelist: allowed".into(), NORMAL)),
                Err(error) => values.push((Outside::error(error).text().into(), ERROR)),
            }
            if !review.sufficient_funds() {
                values.push((
                    "Insufficient funds: no signature will be requested.".into(),
                    WARNING,
                ));
            }
            values.push((
                format!("Complete calldata ({} bytes):", intent.input().len()),
                HINT,
            ));
            // The review owns these decoded bytes. Hex and exact integer strings are ASCII;
            // split them into bounded rows without truncating an argument or a final digit.
            values.push((format!("0x{}", alloy::hex::encode(intent.input())), NORMAL));
            values.push(("End of reviewed calldata".into(), HINT));
            let width = usize::from(inner.width).clamp(1, 72);
            let lines: Vec<_> = values
                .into_iter()
                .flat_map(|(text, style)| {
                    text.as_bytes()
                        .chunks(width)
                        .map(|part| (String::from_utf8_lossy(part).into_owned(), style))
                        .collect::<Vec<_>>()
                })
                .collect();
            let room = usize::from(inner.height.saturating_sub(2));
            let offset = state.review_scroll.min(lines.len().saturating_sub(room));
            for (y, (text, style)) in lines.iter().skip(offset).take(room).enumerate() {
                row(
                    frame,
                    y as u16,
                    "",
                    Some(Outside::detail(text)),
                    *style,
                    placed,
                );
            }
            let y = inner.height.saturating_sub(2);
            if state.stage == Stage::Confirm {
                row(
                    frame,
                    y,
                    "Type the contract nickname exactly:",
                    None,
                    TITLE,
                    placed,
                );
                field(frame, inner, y + 1, app, placed);
            } else {
                row(
                    frame,
                    y,
                    "j/k scroll the complete calldata; Home returns to the beginning.",
                    None,
                    HINT,
                    placed,
                );
                row(
                    frame,
                    y + 1,
                    "Enter opens a separate nickname confirmation.",
                    None,
                    HINT,
                    placed,
                );
            }
        }
        Stage::Result => match &state.result {
            Some(ToolResult::Call(result)) => {
                let text = if state.alternate {
                    &result.raw
                } else {
                    &result.rendered_result
                };
                wrapped(frame, inner, &Outside::input(text), NORMAL, placed);
            }
            Some(ToolResult::Signature(result)) => {
                let label = if state.alternate {
                    "digest"
                } else {
                    "signature"
                };
                row(frame, 0, label, None, TITLE, placed);
                let area = Rect {
                    y: inner.y + 2,
                    height: inner.height.saturating_sub(2),
                    ..inner
                };
                wrapped(
                    frame,
                    area,
                    &Outside::input(result[label].as_str().unwrap_or("invalid result")),
                    NORMAL,
                    placed,
                );
            }
            Some(ToolResult::Token {
                balance,
                owner,
                token,
            }) => {
                row(
                    frame,
                    0,
                    "Account: ",
                    Some(Outside::address(owner)),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    1,
                    "Contract: ",
                    Some(Outside::address(token)),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    3,
                    "Symbol: ",
                    Some(Outside::input(&balance.symbol)),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    4,
                    &format!("Decimals: {}", balance.decimals),
                    None,
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    6,
                    "Balance: ",
                    Some(Outside::quantity(&format_units(
                        balance.raw,
                        balance.decimals,
                    ))),
                    NORMAL,
                    placed,
                );
                row(frame, 8, "Raw units:", None, HINT, placed);
                row(
                    frame,
                    9,
                    "",
                    Some(Outside::quantity(&balance.raw.to_string())),
                    NORMAL,
                    placed,
                );
                row(
                    frame,
                    12,
                    "Read only. This does not add the token to the send registry.",
                    None,
                    HINT,
                    placed,
                );
            }
            None => {}
        },
        Stage::Error => {
            if let Some(error) = &state.notice {
                wrapped(frame, inner, error, ERROR, placed);
            }
        }
    }
}
fn field(frame: &mut Frame, inner: Rect, y: u16, app: &App, placed: &mut Vec<Placed>) {
    if let Some(field) = &app.tools().field {
        let width = usize::from(inner.width.saturating_sub(2));
        let display = field.drawn();
        let offset = field.cursor().saturating_sub(width.saturating_sub(1));
        let shown = &display[offset.min(display.len())..display.len().min(offset + width)];
        Row::default()
            .text("> ", TITLE)
            .outside(&Outside::input(shown), NORMAL)
            .draw(frame, line(inner, y), placed);
        frame.set_cursor_position((
            inner.x
                + 2
                + field
                    .cursor()
                    .saturating_sub(offset)
                    .min(width.saturating_sub(1)) as u16,
            inner.y + y,
        ));
    }
}
fn wrapped(frame: &mut Frame, area: Rect, value: &Outside, style: Style, placed: &mut Vec<Placed>) {
    // This entire area is one source. Input is already ASCII and bounded at entry.
    let text = value.text();
    let width = usize::from(area.width.max(1));
    for (i, bytes) in text
        .as_bytes()
        .chunks(width)
        .take(usize::from(area.height))
        .enumerate()
    {
        let row = Rect {
            y: area.y + i as u16,
            height: 1,
            ..area
        };
        let part = Outside::input(std::str::from_utf8(bytes).expect("inert text is ASCII"));
        Row::default()
            .outside(&part, style)
            .draw(frame, row, placed);
    }
}

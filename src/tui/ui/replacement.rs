//! Replacement review from immutable decoded bytes, with no editable nonce or recipient.
use super::*;
use crate::core::amount::format_units;
use crate::tui::app::replacement::Stage;
use crate::wallet::replacement::{Action, CANCEL_RACE};

pub(super) fn pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let border = Block::bordered().border_set(ASCII).title(" replacement ");
    let inner = border.inner(area);
    frame.render_widget(border, area);
    let state = app.replacement();
    let mut lines: Vec<String> = Vec::new();
    if let Some(review) = &state.review {
        let original = review.original();
        let tx = review.intent();
        lines.push(format!(
            "{} on {} at nonce {}",
            match review.action() {
                Action::SpeedUp => "Speed up",
                Action::Cancel => "Cancel",
                Action::Retry => "Retry",
            },
            original.chain().key,
            tx.nonce()
        ));
        lines.push("Original hash:".into());
        lines.push(review.replaced_hash());
        lines.push(format!("From: {}", original.from()));
        lines.push(format!("To: {}", tx.to().to_checksum(None)));
        let decoded = tx.action().to_json();
        lines.push(format!(
            "Action: {}",
            decoded["kind"].as_str().unwrap_or("unknown")
        ));
        if let Some(to) = decoded["to"].as_str() {
            lines.push(format!("Payee: {to}"));
        }
        if let Some(token) = decoded["token"].as_str() {
            lines.push(format!("Token: {token}"));
        }
        if let Some(amount) = decoded["amount"].as_str() {
            lines.push(format!("Token base units: {amount}"));
        }
        if let Some(selector) = decoded["selector"].as_str() {
            lines.push(format!("Selector: {selector}"));
        }
        lines.push(format!(
            "Native value: {} {}",
            format_units(tx.value(), original.chain().decimals),
            original.chain().symbol
        ));
        lines.push(format!("Native base units: {}", tx.value()));
        lines.push(format!(
            "Bump: {}",
            crate::chains::bump_label(review.options().bump)
        ));
        lines.push(format!(
            "Maximum execution fee: {} {}",
            format_units(review.max_gas_cost(), original.chain().decimals),
            original.chain().symbol
        ));
        if let Some(l1) = review.l1_fee() {
            lines.push(format!(
                "L1 estimate: {} {}",
                format_units(l1, original.chain().decimals),
                original.chain().symbol
            ));
        }
        lines.push(format!(
            "Maximum reviewed total: {} {}",
            format_units(review.max_total_fee(), original.chain().decimals),
            original.chain().symbol
        ));
        lines.push(format!("Maximum total in wei: {}", review.max_total_fee()));
        lines.push(format!(
            "Funds: {}",
            if review.sufficient_funds() {
                "sufficient"
            } else {
                "insufficient"
            }
        ));
        lines.push(format!(
            "Policy: {}",
            match review.verdict() {
                Ok(()) => "allowed".into(),
                Err(e) => e.message.clone(),
            }
        ));
        lines.push(format!("Confirm by typing: {}", review.confirmation()));
        if review.is_cancel() {
            lines.extend(wrapped(CANCEL_RACE, usize::from(inner.width)));
        }
        if !tx.input().is_empty() {
            lines.push(format!("Complete calldata ({} bytes):", tx.input().len()));
            lines.push(format!("0x{}", alloy::hex::encode(tx.input())));
        }
    } else if state.stage == Stage::Preparing {
        lines.push("Checking signed original, current policy, nonce and fees...".into());
    }
    if let Some(notice) = &state.notice {
        lines.extend(wrapped(notice.text(), usize::from(inner.width)));
    }
    // Preserve every digit and calldata byte across scrollable rows. Detail's 72-character
    // bound applies to each row, not to the entire reviewed quantity or calldata field.
    let width = usize::from(inner.width).clamp(1, 72);
    let lines: Vec<String> = lines
        .into_iter()
        .flat_map(|value| {
            let inert = crate::core::text::ascii_inert(&value, 262_200);
            inert
                .as_bytes()
                .chunks(width)
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let field = state.field.as_ref();
    let room = usize::from(inner.height).saturating_sub(usize::from(field.is_some()) * 2);
    let offset = state.scroll.min(lines.len().saturating_sub(room));
    for (y, value) in lines.iter().skip(offset).take(room).enumerate() {
        Row::default()
            .outside(
                &Outside::detail(value).cut(usize::from(inner.width)),
                NORMAL,
            )
            .draw(frame, line(inner, y as u16), placed);
    }
    if let Some(field) = field {
        let y = inner.height.saturating_sub(2);
        let label = if state.stage == Stage::Bump {
            "Bump (1.1 to 3):"
        } else {
            "Type the reviewed nickname or cancel:"
        };
        Row::default()
            .text(label, HINT)
            .draw(frame, line(inner, y), placed);
        Row::default()
            .outside(
                &Outside::detail(&field.drawn()).cut(usize::from(inner.width)),
                SELECTED,
            )
            .draw(frame, line(inner, y + 1), placed);
    }
}

fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            result.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        result.push(line);
    }
    result
}

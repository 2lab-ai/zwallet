//! Task reports are untrusted observations. Every row passes Outside and the terminal filter.
use super::*;
use crate::tui::app::tasks::Stage;

pub(super) fn pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let state = app.tasks();
    let block = Block::bordered()
        .border_set(ASCII)
        .title(" tasks / advisory accounting ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec!["Local records and RPC observations; not a spending limit.".to_string()];
    match state.stage {
        Stage::List => {
            lines.push("Account addresses hidden; Enter reconciles the selected task.".into());
            if state.rows.is_empty() {
                lines.push("No tasks. Press n to create one for the open key.".into());
            }
            let room = usize::from(inner.height.saturating_sub(3)) / 2;
            let offset = state.selected.saturating_sub(room.saturating_sub(1));
            for (i, task) in state.rows.iter().enumerate().skip(offset).take(room) {
                lines.push(format!(
                    "{} {} / {}",
                    if i == state.selected { ">" } else { " " },
                    task.id,
                    task.chain
                ));
                lines.push(format!(
                    "  {} intents / created {} UTC epoch",
                    task.intent_count, task.created_at
                ));
            }
        }
        Stage::CreateId => lines
            .push("Task ID: 1-32 lowercase letters/digits/_/-; no descriptions or secrets.".into()),
        Stage::Allocations => {
            lines.push("Optional allocations in exact base units; comma separated.".into());
            lines.push("native=1000,REGISTRY_TOKEN_CONTRACT=2000 (blank: no allocation)".into());
            lines.push("Allocations are report labels, never enforced budgets.".into());
        }
        Stage::ConfirmCreate => {
            if let Some(draft) = &state.draft {
                lines.extend([
                    format!("Task: {}", draft.id),
                    format!(
                        "Selected key: {} / chain {}",
                        draft.key,
                        app.net().chain.key
                    ),
                    "Baseline will be read from this session's RPC; no funds move.".into(),
                ]);
                for (asset, amount) in &draft.allocations {
                    lines.push(format!("{asset} = {amount} base units"));
                }
            }
        }
        Stage::ReportId => {
            lines.push("Task ID to report (its recorded chain chooses the session RPC).".into())
        }
        Stage::Loading => lines
            .push("Reading / saving... previous report cleared until this read completes.".into()),
        Stage::Error => lines.push("Task operation failed; Enter returns to the task list.".into()),
        Stage::Report => {
            if let Some(report) = &state.report {
                lines.extend([
                    format!(
                        "Task {} / {} / observed {} UTC epoch",
                        report.task_id, report.chain, report.observed_at
                    ),
                    format!(
                        "Source: {} / addresses {}",
                        if report.offline {
                            "local records only; outcomes unverified"
                        } else {
                            "local records + session RPC observations"
                        },
                        if state.reveal {
                            "revealed by request"
                        } else {
                            "redacted"
                        }
                    ),
                    format!("Owner: {}", report.owner),
                ]);
                lines.push(format!(
                    "Observation began {} UTC epoch",
                    report.observation_started_at
                ));
                lines.push(format!(
                    "Baseline source {} / observed {} UTC epoch",
                    report.baseline_source, report.baseline_observed_at
                ));
                lines.push(format!("Current source: {}", known(&report.current_source)));
                for warning in &report.warnings {
                    lines.push(format!("Warning: {warning}"));
                }
                for asset in &report.assets {
                    lines.extend([
                        format!(
                            "Asset {} ({}) / {} decimals; exact base units",
                            asset.asset, asset.symbol, asset.decimals
                        ),
                        format!(
                            "Allocation: {} / remaining: {}",
                            known(&asset.allocation_raw),
                            known(&asset.remaining_allocation_raw)
                        ),
                        format!(
                            "Baseline: {} / current: {} / change: {}",
                            known(&asset.baseline_balance_raw),
                            known(&asset.current_balance_raw),
                            known(&asset.balance_change_raw)
                        ),
                        format!(
                            "Confirmed direct intent: {} / known native fees: {}",
                            asset.confirmed_intent_raw, asset.known_fees_raw
                        ),
                        format!(
                            "Accounting complete: {}; fees/assets never summed across assets",
                            asset.accounting_complete
                        ),
                    ]);
                }
                for record in &report.records {
                    lines.extend([
                        format!(
                            "Intent {}: {:?} / {}",
                            record.intent_id, record.outcome, record.kind
                        ),
                        format!(
                            "{} {} base units -> {}",
                            record.amount_raw, record.asset, record.recipient
                        ),
                        format!(
                            "Tx {} / nonce {}",
                            known(&record.tx_hash),
                            record.nonce.map_or_else(|| "n/a".into(), |n| n.to_string())
                        ),
                        format!(
                            "Replaces {} / same-nonce attempts {}",
                            known(&record.replaces),
                            record.same_nonce_attempts.len()
                        ),
                        format!(
                            "Actual native fee: {} / calldata hash: {}",
                            known(&record.actual_fee_raw),
                            known(&record.calldata_hash)
                        ),
                    ]);
                    for hash in &record.same_nonce_attempts {
                        lines.push(format!("Same-nonce attempt: {hash}"));
                    }
                    for evidence in &record.evidence {
                        lines.push(format!("Evidence: {evidence}"));
                    }
                }
                lines.push("End of report. Unknown accounting stays unknown; a receipt is not finality proof.".into());
            }
        }
        Stage::Closed => {}
    }
    let width = usize::from(inner.width).clamp(1, 72);
    let rows: Vec<_> = lines
        .iter()
        .flat_map(|s| {
            let safe = crate::core::text::ascii_inert(s, 8192);
            safe.as_bytes()
                .chunks(width)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let room = usize::from(
        inner
            .height
            .saturating_sub(if state.field.is_some() { 2 } else { 0 }),
    );
    let offset = state.scroll.min(rows.len().saturating_sub(room));
    for (y, text) in rows.iter().skip(offset).take(room).enumerate() {
        Row::default().outside(&Outside::detail(text), NORMAL).draw(
            frame,
            line(inner, y as u16),
            placed,
        );
    }
    if let Some(field) = &state.field {
        let width = usize::from(inner.width).saturating_sub(2);
        let start = field.cursor().saturating_sub(width.saturating_sub(1));
        let text: String = field.drawn().chars().skip(start).take(width).collect();
        let cursor = field.cursor().saturating_sub(start);
        let y = inner.height.saturating_sub(1);
        Row::default()
            .text("> ", TITLE)
            .outside(&Outside::input(&text), NORMAL)
            .draw(frame, line(inner, y), placed);
        frame.set_cursor_position((inner.x + 2 + cursor as u16, inner.y + y));
    }
}
fn known(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("unknown")
}

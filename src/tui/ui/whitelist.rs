//! Whitelist rendering: every outside value is inert; exact summaries wrap without omission.
use super::*;
use crate::tui::app::whitelist::{Stage, lines};

pub(super) fn active(app: &App) -> bool {
    app.tab() == Tab::Whitelist
        && app.navigation().popup.is_none()
        && (app.password_state() == PasswordState::Idle
            || matches!(
                app.password_purpose(),
                PasswordPurpose::WhitelistVerify | PasswordPurpose::WhitelistEdit
            ))
}
pub(super) fn hint(app: &App) -> &'static str {
    if app.whitelist_busy() {
        return "Saving policy... L locks afterward; q/Ctrl-C quits after pending work";
    }
    if matches!(app.password_state(), PasswordState::Input { .. }) {
        return "Enter apply/check   Esc cancel   password input is never echoed";
    }
    match &app.whitelist_ui().stage {
        Stage::List => {
            "a add C contract d remove f fee n/t/b caps m msgs R reset V verify ? actions"
        }
        Stage::Form(_) => "Tab/Up/Down fields   Enter next/review   Esc cancel   Ctrl-C quit",
        Stage::Review(_) => {
            "Up/Down/Home/End review exact summary   Enter next page/password   Esc cancel"
        }
        Stage::Outcome { .. } => "Up/Down scroll   Enter/Esc back to list   q quit",
        _ => "Preparing the exact policy summary...",
    }
}
fn draw_lines(
    frame: &mut Frame,
    area: Rect,
    values: &[Outside],
    scroll: usize,
    placed: &mut Vec<Placed>,
) {
    for (row, text) in values
        .iter()
        .skip(scroll)
        .take(usize::from(area.height))
        .enumerate()
    {
        Row::default()
            .outside(text, NORMAL)
            .draw(frame, line(area, row as u16), placed);
    }
}
/// The selected entry's full fields, which its list row may cut at the pane's width: each on
/// rows of its own, short enough for 80 columns (rule 4: list rows are cut, never wrapped).
fn selected_fields(entry: &crate::core::whitelist::Entry) -> Vec<Outside> {
    let address = if entry.chains == ["tron"] {
        alloy::hex::decode(entry.address.trim_start_matches("0x"))
            .ok()
            .and_then(|body| <[u8; 20]>::try_from(body).ok())
            .map(|body| crate::chains::tron::address::from_body(&body))
            .unwrap_or_else(|| entry.address.clone())
    } else {
        crate::chains::evm::parse_address(&entry.address)
            .map(|a| a.to_checksum(None))
            .unwrap_or_else(|_| entry.address.clone())
    };
    let mut rows = vec![Outside::detail(&format!(
        "  {} {address}",
        entry.kind.as_str()
    ))];
    rows.extend(
        entry
            .chains
            .chunks(5)
            .map(|chains| Outside::detail(&format!("  chains {}", chains.join(", ")))),
    );
    if let Some(selectors) = &entry.selectors {
        rows.extend(
            selectors
                .chunks(4)
                .map(|s| Outside::detail(&format!("  selectors {}", s.join(", ")))),
        );
    }
    rows
}
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let block = Block::bordered().border_set(ASCII).title(" whitelist ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.password_purpose() == PasswordPurpose::WhitelistVerify
        && app.password_state() != PasswordState::Idle
    {
        Row::default()
            .text(
                "Verify local list with the password; no session or file changes",
                TITLE,
            )
            .draw(frame, line(inner, 0), placed);
        password(
            frame,
            Rect::new(
                inner.x,
                inner.y + 2,
                inner.width,
                inner.height.saturating_sub(2),
            ),
            app,
            placed,
        );
        return;
    }
    match &app.whitelist_ui().stage {
        Stage::List => {
            let mut header = Vec::new();
            if let Some(checked) = app.checked_list() {
                header.extend(lines(&format!(
                    "verified by password: gen {}, {} entries, source {}",
                    checked.list.generation,
                    checked.list.entries.len(),
                    checked.source
                )));
                if let Some(entry) = checked.list.entries.get(app.whitelist_ui().selected) {
                    header.extend(lines(&format!(
                        "selected for d: {} ({})",
                        entry.nickname,
                        if crate::wallet::whitelist::is_zero_address(entry) {
                            "zero address; unavailable as payee"
                        } else {
                            "locally verified"
                        }
                    )));
                    header.extend(selected_fields(entry));
                }
            }
            // This is fixed chrome, not a value supplied by a file or daemon.
            let unverified = usize::from(app.checked_list().is_none());
            if let Some(session) = &app.navigation().session
                && let Some(serving) = &session.serving
            {
                // Only counts and a number from the session; never names, addresses or its claim.
                let generation = serving.whitelist["gen"]
                    .as_u64()
                    .map(|g| g.to_string())
                    .unwrap_or_else(|| "unknown".into());
                let count = serving.whitelist["entries"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                header.extend(lines(&format!("running session reports gen {generation}, {count} entries; this TUI did not check it")));
            }
            if !app.whitelist_ui().next_unlock.is_empty() {
                header.extend(lines(&format!(
                    "next edit only: unlock {} (confirmed in its summary)",
                    app.whitelist_ui().next_unlock
                )));
            }
            let height = u16::try_from(header.len() + unverified)
                .unwrap_or(u16::MAX)
                .min(inner.height);
            if unverified != 0 {
                Row::default()
                    .text(
                        "not verified: V checks this list locally before selecting entries",
                        NORMAL,
                    )
                    .draw(frame, line(inner, 0), placed);
            }
            draw_lines(
                frame,
                Rect {
                    y: inner.y + unverified as u16,
                    height: height.saturating_sub(unverified as u16),
                    ..inner
                },
                &header,
                0,
                placed,
            );
            let body = Rect::new(
                inner.x,
                inner.y + height,
                inner.width,
                inner.height.saturating_sub(height),
            );
            match &app.whitelist_ui().lines {
                Pane::Ready(values) => {
                    draw_lines(frame, body, values, app.whitelist_ui().scroll, placed)
                }
                Pane::Loading => Row::default().text("checking list files...", HINT).draw(
                    frame,
                    line(body, 0),
                    placed,
                ),
                Pane::Failed(error) => {
                    error_view(frame, body, "whitelist cannot be read:", error, placed)
                }
            }
        }
        Stage::Form(form) => {
            Row::default()
                .text(form.title, TITLE)
                .draw(frame, line(inner, 0), placed);
            if let Some(nick) = &form.remove {
                Row::default()
                    .text("remove: ", NORMAL)
                    .outside(&Outside::detail(nick), NORMAL)
                    .draw(frame, line(inner, 1), placed);
            }
            for (row, value) in form.rows.iter().enumerate() {
                let y = 2 + row * 2;
                if y + 1 >= usize::from(inner.height) {
                    break;
                }
                let style = if form.selected == row {
                    SELECTED
                } else {
                    NORMAL
                };
                Row::default()
                    .text(if form.selected == row { "> " } else { "  " }, style)
                    .text(value.label, style)
                    .draw(frame, line(inner, y as u16), placed);
                let drawn = value.field.drawn();
                let width = usize::from(inner.width);
                let cursor = value.field.cursor().min(drawn.len());
                let start = if form.selected == row {
                    cursor.saturating_sub(width.saturating_sub(1))
                } else {
                    0
                };
                let end = (start + width).min(drawn.len());
                Row::default()
                    .outside(&Outside::input(&drawn[start..end]), style)
                    .draw(frame, line(inner, y as u16 + 1), placed);
                if form.selected == row {
                    frame.set_cursor_position((
                        inner.x + (cursor - start) as u16,
                        inner.y + y as u16 + 1,
                    ));
                }
            }
            let note = form.error.as_ref().cloned().unwrap_or_else(|| {
                Outside::detail(
                    "chains: comma-separated; blank = address family default; * is EVM only",
                )
            });
            Row::default()
                .outside(&note, if form.error.is_some() { ERROR } else { HINT })
                .draw(frame, line(inner, inner.height.saturating_sub(1)), placed);
        }
        Stage::Preparing => {
            Row::default()
                .text(
                    "Preparing summary; nothing has been stopped or changed",
                    HINT,
                )
                .draw(frame, line(inner, 0), placed);
        }
        Stage::Review(review) | Stage::Applying(review) => {
            let reserve = if app.password_state() == PasswordState::Idle {
                2
            } else {
                5
            };
            let summary_area = Rect {
                height: inner.height.saturating_sub(reserve),
                ..inner
            };
            draw_lines(frame, summary_area, &review.lines, review.scroll, placed);
            let foot = Rect::new(inner.x, inner.y + summary_area.height, inner.width, reserve);
            if app.password_state() == PasswordState::Idle {
                Row::default()
                    .text(
                        &format!(
                            "Summary rows {}-{} of {} (no text omitted)",
                            review.scroll + 1,
                            (review.scroll + usize::from(summary_area.height))
                                .min(review.lines.len()),
                            review.lines.len()
                        ),
                        HINT,
                    )
                    .draw(frame, line(foot, 0), placed);
                Row::default()
                    .text(
                        "Enter pages through every row, then asks for the password",
                        HINT,
                    )
                    .draw(frame, line(foot, 1), placed);
            } else {
                password(frame, foot, app, placed);
            }
        }
        Stage::Outcome { lines, scroll } => draw_lines(frame, inner, lines, *scroll, placed),
    }
}
fn password(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let label = match app.password_state() {
        PasswordState::Preparing => "preparing...",
        PasswordState::Checking => "Saving/checking... wait for the outcome",
        _ => "Wallet password (new for this operation):",
    };
    Row::default()
        .text(label, TITLE)
        .draw(frame, line(area, 0), placed);
    if let PasswordState::Input {
        characters,
        rejected,
    } = app.password_state()
    {
        // Count only: no secret byte has entered App, Msg or any rendering value.
        let stars = "*".repeat(characters.min(usize::from(area.width)));
        Row::default()
            .text(&stars, NORMAL)
            .draw(frame, line(area, 1), placed);
        if rejected {
            Row::default()
                .text("input refused; type it again", ERROR)
                .draw(frame, line(area, 2), placed);
        }
    }
    if app.navigation().allow_ptrace_scope_0 && area.height > 3 {
        Row::default()
            .text("ptrace_scope 0 accepted", WARNING)
            .draw(frame, line(area, 3), placed);
    }
}

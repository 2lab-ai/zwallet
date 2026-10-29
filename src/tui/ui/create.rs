//! The creation form and public result. This module cannot access any password buffer.
use super::*;
use crate::tui::app::create::{BACKUP_HINT, CreateStage};

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let state = app.creation();
    let block = Block::bordered().border_set(ASCII).title(" create a key ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match &state.stage {
        CreateStage::Idle => {}
        CreateStage::Form => {
            let form = state.form.as_ref().expect("form state");
            Row::default()
                .text(
                    if form.first {
                        "Create the first wallet and key 0."
                    } else {
                        "Create another key from this wallet's seed."
                    },
                    TITLE,
                )
                .draw(frame, line(inner, 0), placed);
            Row::default()
                .text(
                    "The new key becomes active for the agent. No recovery phrase is shown.",
                    HINT,
                )
                .draw(frame, line(inner, 1), placed);
            for (index, label) in [
                (0, "Key name: "),
                (1, "Words (Left/Right): "),
                (2, "Unlock duration (blank = stay locked): "),
                (3, "Confirm by typing the key name: "),
            ] {
                if index == 1 && !form.first {
                    continue;
                }
                let style = if form.focus == index {
                    SELECTED
                } else {
                    NORMAL
                };
                Row::default()
                    .text(label, style)
                    .outside(&Outside::input(&form.fields[index].drawn()), style)
                    .draw(frame, line(inner, index as u16 + 3), placed);
            }
            if state.mutation_failed {
                Row::default()
                    .text(
                        "A key may already exist. Esc returns to the refreshed key list.",
                        WARNING,
                    )
                    .draw(frame, line(inner, 8), placed);
            } else if app.password_state() == PasswordState::Failed {
                Row::default()
                    .text(
                        "Preparation refused. Enter/Esc dismisses before editing again.",
                        HINT,
                    )
                    .draw(frame, line(inner, 8), placed);
            }
        }
        CreateStage::Preparing => Row::default()
            .text(
                "Checking the name, wallet and optional signing session...",
                HINT,
            )
            .draw(frame, line(inner, 0), placed),
        CreateStage::PasswordFirst | CreateStage::PasswordAgain | CreateStage::PasswordExisting => {
            let text = match state.stage {
                CreateStage::PasswordFirst => "New wallet password (at least 12 characters):",
                CreateStage::PasswordAgain => "Repeat the new wallet password:",
                _ => "Wallet password: verify and create this named key",
            };
            Row::default()
                .text(text, TITLE)
                .draw(frame, line(inner, 0), placed);
            if app.navigation().allow_ptrace_scope_0 {
                Row::default()
                    .text("ptrace_scope 0 accepted", WARNING)
                    .draw(frame, line(inner, 1), placed);
            }
            if let PasswordState::Input {
                characters,
                rejected,
            } = app.password_state()
            {
                let mask = "*".repeat(characters);
                for (y, chunk) in mask
                    .as_bytes()
                    .chunks(usize::from(inner.width))
                    .enumerate()
                    .take(usize::from(inner.height.saturating_sub(5)))
                {
                    Row::default()
                        .text(std::str::from_utf8(chunk).expect("mask"), NORMAL)
                        .draw(frame, line(inner, y as u16 + 2), placed);
                }
                if rejected {
                    Row::default().text("Input refused and cleared; a new password needs at least 12 characters.",ERROR).draw(frame,line(inner,inner.height-2),placed);
                }
            }
        }
        CreateStage::Submitting => Row::default()
            .text(
                "Creating... this write finishes before quitting or locking.",
                HINT,
            )
            .draw(frame, line(inner, 0), placed),
        CreateStage::Done(created) => {
            let suffix = format!(" at index {} (active, verified by password)", created.index);
            let room = usize::from(inner.width).saturating_sub(8 + suffix.len());
            Row::default()
                .text("Created ", TITLE)
                .outside(&Outside::key_name(&created.name).cut(room), TITLE)
                .text(&suffix, TITLE)
                .draw(frame, line(inner, 0), placed);
            for (y, (chain, address)) in created
                .addresses
                .iter()
                .enumerate()
                .take(usize::from(inner.height.saturating_sub(6)))
            {
                Row::default()
                    .text(&format!("{chain}: "), NORMAL)
                    .outside(&Outside::address(address), NORMAL)
                    .draw(frame, line(inner, y as u16 + 2), placed);
            }
            if created.master_created {
                Row::default()
                    .text("Back up this new wallet before funding it.", WARNING)
                    .draw(frame, line(inner, inner.height - 3), placed);
            }
            Row::default().text(BACKUP_HINT, HINT).draw(
                frame,
                line(inner, inner.height - 2),
                placed,
            );
        }
    }
    if let Some(error) = &state.notice {
        let at = Rect {
            y: inner.y + 9,
            height: inner.height.saturating_sub(9).min(7),
            ..inner
        };
        if error.source() == crate::tui::text::Source::Error {
            error_view(
                frame,
                at,
                "operation refused; Esc returns to keys:",
                error,
                placed,
            );
        } else {
            Row::default()
                .outside(&error.cut(usize::from(inner.width)), ERROR)
                .draw(frame, line(inner, inner.height - 1), placed);
        }
    }
}

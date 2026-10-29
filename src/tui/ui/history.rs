//! Bounded history and detail drawing; every outside value is placed as inert text.
use super::*;
use crate::core::amount::format_units;
use crate::history::{Source as HistorySource, State};
use crate::tui::app::history::{detail_lines, state_label};

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let state = app.history();
    let block = Block::bordered()
        .border_set(ASCII)
        .title(if state.detail.is_some() {
            " transaction "
        } else {
            " wallet "
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Pane::Failed(error) = app.keys() {
        error_view(
            frame,
            inner,
            "the read could not be completed:",
            error,
            placed,
        );
        return;
    }
    if let Some((request, pane)) = &state.detail {
        match pane {
            Pane::Failed(error) => {
                error_view(frame, inner, "transaction lookup failed:", error, placed)
            }
            Pane::Loading => draw_lines(
                frame,
                inner,
                &[
                    Outside::detail(&format!("hash: {}", request.hash)),
                    Outside::detail("reading transaction..."),
                ],
                0,
                placed,
            ),
            Pane::Ready(_) => {
                let lines = detail_lines(app, request, pane);
                draw_lines(
                    frame,
                    Rect {
                        height: inner.height.saturating_sub(1),
                        ..inner
                    },
                    &lines,
                    state.scroll,
                    placed,
                );
            }
        }
        Row::default()
            .text("s/x/R unavailable   r refresh   Esc history", HINT)
            .draw(frame, line(inner, inner.height - 1), placed);
        return;
    }
    match &app.navigation().lines {
        Pane::Ready(lines) => draw_lines(
            frame,
            Rect { height: 4, ..inner },
            lines,
            app.navigation().scroll,
            placed,
        ),
        Pane::Loading => {
            Row::default()
                .text("reading balances...", HINT)
                .draw(frame, line(inner, 0), placed)
        }
        Pane::Failed(error) => Row::default()
            .outside(&error.cut(usize::from(inner.width)), ERROR)
            .draw(frame, line(inner, 0), placed),
    }
    Row::default()
        .text(
            if state.balance_focus {
                "Recent transactions (Tab: focus history; sources are claims)"
            } else {
                "Recent transactions (up to 10; sources are claims)"
            },
            TITLE,
        )
        .draw(frame, line(inner, 4), placed);
    match &state.rows {
        Pane::Loading => {
            Row::default()
                .text("reading history...", HINT)
                .draw(frame, line(inner, 5), placed)
        }
        Pane::Failed(error) => error_view(
            frame,
            Rect {
                y: inner.y + 5,
                height: inner.height - 5,
                ..inner
            },
            "history unavailable:",
            error,
            placed,
        ),
        Pane::Ready(history) => {
            for (index, row) in history.rows.iter().take(10).enumerate() {
                let hash = match (
                    row.tx_hash.get(..8),
                    row.tx_hash.get(row.tx_hash.len().saturating_sub(6)..),
                ) {
                    (Some(first), Some(last)) => format!("{first}..{last}"),
                    _ => Outside::detail(&row.tx_hash).text().to_string(),
                };
                let value = if let Some(m) = row.movements.first() {
                    format!("{} {}", format_units(m.amount, m.decimals), m.symbol)
                } else {
                    row.value
                        .map(|n| {
                            format!(
                                "{} {}",
                                format_units(n, app.chain().decimals),
                                app.chain().symbol
                            )
                        })
                        .or_else(|| row.journal.as_ref().map(|j| j.value.clone()))
                        .unwrap_or_else(|| "unknown".into())
                };
                let sources = row
                    .sources
                    .iter()
                    .map(|source| match source {
                        HistorySource::Journal => "journal",
                        HistorySource::Explorer => "explorer",
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let label = if row.state == State::Mined {
                    "mined"
                } else {
                    state_label(row.state, row.success)
                };
                let text = format!(
                    "{:<9} {:<16} {:<17} [{}]",
                    Outside::detail(label).cut(9).text(),
                    Outside::detail(&hash).cut(16).text(),
                    Outside::detail(&value).cut(17).text(),
                    sources
                );
                let style = if index == state.selected && !state.balance_focus {
                    SELECTED
                } else {
                    NORMAL
                };
                Row::default()
                    .outside(&Outside::detail(&text).cut(usize::from(inner.width)), style)
                    .draw(frame, line(inner, index as u16 + 5), placed);
            }
            if history.rows.is_empty() {
                Row::default()
                    .text("No transactions in these sources.", HINT)
                    .draw(frame, line(inner, 5), placed);
            }
            let coverage = Outside::warning(&history.coverage);
            for (offset, chunk) in coverage
                .text()
                .as_bytes()
                .chunks(usize::from(inner.width).min(72))
                .take(2)
                .enumerate()
            {
                Row::default()
                    .outside(&Outside::detail(&String::from_utf8_lossy(chunk)), HINT)
                    .draw(frame, line(inner, inner.height - 2 + offset as u16), placed);
            }
        }
    }
}

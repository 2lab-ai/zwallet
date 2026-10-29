//! Read-only, untrusted history claims. Nothing here resolves a payee, supplies a signing
//! intent, or marks an address verified. A detail must be read from the selected node anew.

pub mod tron;

use crate::chains::evm::explorer::{Page, Unavailable};
use crate::chains::{Chain, JournalRow};
use alloy::primitives::U256;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Journal,
    Explorer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Unknown,
    Pending,
    Mined,
    NotFound,
    DroppedReplaced,
    ExpiredUnseen,
}

/// One registry-token movement, from shaped explorer fields or decoded receipt logs. The
/// symbol and decimals are constants from this build's registry, never an outside label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenMovement {
    pub token: String,
    pub symbol: &'static str,
    pub decimals: u8,
    pub from: String,
    pub to: String,
    pub amount: U256,
    pub log_index: u32,
}

/// A row's strings have been shape checked at their source. Its provenance stays visible:
/// these fields are claims, including the local journal's, not authenticated transaction data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub chain: &'static str,
    pub chain_id: String,
    pub tx_hash: String,
    pub state: State,
    pub block_number: Option<u64>,
    pub position: Option<u32>,
    pub nonce: Option<u64>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub value: Option<U256>,
    pub fee: Option<U256>,
    pub timestamp: Option<u64>,
    pub success: Option<bool>,
    pub movements: Vec<TokenMovement>,
    pub sources: Vec<Source>,
    /// Locally recorded intent, still only a claim. Never use it to authorize a replacement.
    pub journal: Option<JournalRow>,
}

impl Row {
    pub fn new(chain: &Chain, hash: String, source: Source) -> Self {
        Self {
            chain: chain.key,
            chain_id: chain.chain_id_json().to_string(),
            tx_hash: hash,
            state: State::Unknown,
            block_number: None,
            position: None,
            nonce: None,
            from: None,
            to: None,
            value: None,
            fee: None,
            timestamp: None,
            success: None,
            movements: Vec::new(),
            sources: vec![source],
            journal: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub rows: Vec<Row>,
    pub coverage: String,
    pub dropped: usize,
}

/// Merge only the selected chain, by `(chainId, hash)`. The caller must discard a reply from
/// an obsolete UI refresh generation before calling this. Pending rows lead; mined rows sort
/// by block descending and position ascending; unknown journal rows follow deterministically.
/// An explorer assertion can enrich a journal row, never confer authority on either source.
pub fn merge(
    chain: &Chain,
    journal: Vec<JournalRow>,
    explorer: Result<Page, Unavailable>,
) -> History {
    // Track the presence of an explorer transaction separately: a journal also supplies a
    // nonce, so a merged journal/token row cannot stand in for a transaction response.
    let mut by_hash = BTreeMap::<(String, String), (Row, bool)>::new();
    let chain_id = chain.chain_id_json().to_string();
    let mut insert = |row: Row| {
        if row.chain != chain.key || row.chain_id != chain_id {
            return;
        }
        let key = (row.chain_id.clone(), row.tx_hash.to_ascii_lowercase());
        let explorer_transaction = row.sources.contains(&Source::Explorer) && row.nonce.is_some();
        if let Some((old, has_transaction)) = by_hash.get_mut(&key) {
            if explorer_transaction {
                // A transaction supplies one coherent state. Token pages can be older or
                // newer; their sparse metadata must not overwrite or complete this state.
                old.from = row.from;
                old.to = row.to;
                old.nonce = row.nonce;
                old.value = row.value;
                old.fee = row.fee;
                old.block_number = row.block_number;
                old.position = row.position;
                old.timestamp = row
                    .timestamp
                    .or_else(|| old.journal.as_ref().map(|j| j.timestamp));
                old.success = row.success;
                old.state = row.state;
                *has_transaction = true;
            } else if row.sources.contains(&Source::Explorer) && !*has_transaction {
                old.block_number = row.block_number.or(old.block_number);
                old.state = if old.block_number.is_some() {
                    State::Mined
                } else if row.state != State::Unknown {
                    row.state
                } else {
                    old.state
                };
            }
            for movement in row.movements {
                if !old
                    .movements
                    .iter()
                    .any(|m| m.log_index == movement.log_index && m.token == movement.token)
                {
                    old.movements.push(movement);
                }
            }
            for source in row.sources {
                if !old.sources.contains(&source) {
                    old.sources.push(source);
                }
            }
            old.sources.sort();
        } else {
            by_hash.insert(key, (row, explorer_transaction));
        }
    };
    for j in journal {
        if j.chain != chain.key {
            continue;
        }
        let mut row = Row::new(chain, j.tx_hash.clone(), Source::Journal);
        row.nonce = j.nonce;
        row.from = Some(j.from.clone());
        row.to = Some(j.to.clone());
        row.timestamp = Some(j.timestamp);
        if j.kind != "token-transfer" {
            row.value = j.amount_raw;
        }
        row.journal = Some(j);
        insert(row);
    }
    let (dropped, mut coverage) = match explorer {
        Ok(page) => {
            for row in page.rows {
                insert(row);
            }
            (
                page.dropped,
                format!("{}: explorer and this machine's sends", chain.key),
            )
        }
        Err(Unavailable::Unsupported) => (
            0,
            format!(
                "{}: this machine's sends only; incoming transfers not indexed",
                chain.key
            ),
        ),
        Err(e) => (
            0,
            format!(
                "{}: explorer unavailable ({}); this machine's sends only",
                chain.key,
                e.reason()
            ),
        ),
    };
    if dropped > 0 {
        coverage.push_str(&format!(
            "; {dropped} explorer rows not shown: not in the shape zwallet reads"
        ));
    }
    let mut rows = by_hash
        .into_values()
        .map(|(row, _)| row)
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        (a.state != State::Pending)
            .cmp(&(b.state != State::Pending))
            .then_with(|| b.block_number.cmp(&a.block_number))
            .then_with(|| {
                a.position
                    .unwrap_or(u32::MAX)
                    .cmp(&b.position.unwrap_or(u32::MAX))
            })
            .then_with(|| b.timestamp.cmp(&a.timestamp))
            .then_with(|| a.tx_hash.cmp(&b.tx_hash))
    });
    rows.truncate(10);
    History {
        rows,
        coverage,
        dropped,
    }
}

//! Native Solana/TON review and confirmation. The review owns the selected key, exact-chain
//! password-checked payee and decoded amount. Confirmation never accepts replacement fields.
use super::{
    CheckedList,
    send::{SendOptions, SendOutput, send_prepared, send_summary},
};
use crate::{
    chains::{
        self, Chain, ChainFamily, Family, Net, PreparedTx, TransferRequest, policy::ListView,
    },
    core::{
        amount::parse_units,
        error::{ErrorCode, Result, WalletError},
        whitelist::{self, EntryKind, Fingerprint},
    },
};
use alloy::{hex, primitives::U256};
use serde_json::{Map, Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct NativeCheckedPayee {
    nickname: String,
    address: String,
    chain: &'static str,
    fingerprint: Fingerprint,
}
impl NativeCheckedPayee {
    pub fn nickname(&self) -> &str {
        &self.nickname
    }
    pub fn address(&self) -> &str {
        &self.address
    }
}

fn require_native(chain: &Chain) -> Result<()> {
    if matches!(chain.family, Family::Solana(_) | Family::Ton(_)) {
        Ok(())
    } else {
        Err(WalletError::new(
            ErrorCode::CapabilityUnsupported,
            "this native review is for Solana and TON",
        ))
    }
}
fn normalize(chain: &Chain, address: &str) -> Result<String> {
    match chain.family {
        Family::Solana(_) => chains::solana::normalize_address(address),
        Family::Ton(_) => chains::ton::normalize_address(address),
        _ => Err(WalletError::validation("unsupported native family")),
    }
}
fn from_policy(chain: &Chain, stored: &str) -> Option<String> {
    let shown = match chain.family {
        Family::Solana(_) => {
            let bytes = hex::decode(stored.strip_prefix("solana:")?).ok()?;
            if bytes.len() != 32 {
                return None;
            }
            bs58::encode(bytes).into_string()
        }
        Family::Ton(_) => stored.strip_prefix("ton:")?.to_string(),
        _ => return None,
    };
    normalize(chain, &shown).ok()
}
/// Call `check_unchanged` before showing these choices. Wildcards and contracts never appear.
pub fn native_transfer_payees(
    checked: &CheckedList,
    chain: &'static Chain,
) -> Vec<NativeCheckedPayee> {
    checked
        .list
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Address && e.chains == [chain.key])
        .filter_map(|e| {
            Some(NativeCheckedPayee {
                nickname: e.nickname.clone(),
                address: from_policy(chain, &e.address)?,
                chain: chain.key,
                fingerprint: checked.fingerprint,
            })
        })
        .collect()
}
#[derive(Clone, Debug)]
pub struct NativeTransferInput {
    pub task_id: Option<String>,
    /// The selected password-pinned key, never a fresh daemon status or metadata row.
    pub key_name: String,
    pub index: u32,
    pub from: String,
    pub payee: NativeCheckedPayee,
    pub amount: String,
    pub allow_duplicate: bool,
}
#[derive(Debug)]
pub struct NativeTransferReview {
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: NativeTransferInput,
    request: TransferRequest,
    prepared: PreparedTx,
    decoded: Value,
    verdict: Result<()>,
    fields: Map<String, Value>,
    sufficient: bool,
}
impl NativeTransferReview {
    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
    pub fn from(&self) -> &str {
        &self.input.from
    }
    pub fn key_name(&self) -> &str {
        &self.input.key_name
    }
    pub fn index(&self) -> u32 {
        self.input.index
    }
    pub fn payee(&self) -> &NativeCheckedPayee {
        &self.input.payee
    }
    pub fn verdict(&self) -> &Result<()> {
        &self.verdict
    }
    pub fn sufficient_funds(&self) -> bool {
        self.sufficient
    }
    /// On TON this is an RPC estimate, not an encoded protocol fee maximum.
    pub fn max_fee(&self) -> U256 {
        self.prepared.max_gas_cost
    }
    pub fn fee_cap(&self) -> Option<U256> {
        self.checked.list.policy.max_fee_wei(self.net.chain.key)
    }
    pub fn fingerprint(&self) -> Fingerprint {
        self.checked.fingerprint
    }
    pub fn chain(&self) -> &'static Chain {
        self.net.chain
    }
    pub fn allow_duplicate(&self) -> bool {
        self.input.allow_duplicate
    }
    fn matches(&self, next: &Self) -> bool {
        self.input.key_name == next.input.key_name && self.input.index == next.input.index
            && self.input.from == next.input.from && self.input.payee.address == next.input.payee.address
            && self.input.payee.chain == next.input.payee.chain && self.input.amount == next.input.amount
            && self.verdict == next.verdict && self.sufficient == next.sufficient
            && next.max_fee() <= self.max_fee()
            // An advancing TON sequence can represent another payment. Require new review;
            // a Solana blockhash refresh alone does not alter the held payment.
            && self.decoded.get("seqno") == next.decoded.get("seqno")
    }
}
#[derive(Debug)]
pub enum NativeSendOutcome {
    Sent(SendOutput),
    ReviewChanged(Box<NativeTransferReview>),
}
fn base_fields(chain: &Chain, input: &NativeTransferInput) -> Map<String, Value> {
    json!({"chain":chain.key,"key":input.key_name,"from":input.from,
        "to":input.payee.address,"toNickname":input.payee.nickname})
    .as_object()
    .unwrap()
    .clone()
}
async fn prepare(net: &Net, request: &TransferRequest) -> Result<PreparedTx> {
    tokio::time::timeout(Duration::from_secs(30), async {
        match net.chain.family {
            Family::Solana(_) => chains::solana::Solana.prepare_transfer(net, request).await,
            Family::Ton(_) => chains::ton::Ton.prepare_transfer(net, request).await,
            _ => Err(WalletError::validation("unsupported native family")),
        }
    })
    .await
    .map_err(|_| WalletError::rpc("native transfer preparation timed out; nothing signed"))?
}
fn assemble(
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: NativeTransferInput,
    request: TransferRequest,
    prepared: PreparedTx,
) -> Result<NativeTransferReview> {
    let decoded = match net.chain.family {
        Family::Solana(_) => chains::solana::inspect_unsigned(net.chain, &prepared.unsigned)?,
        Family::Ton(_) => chains::ton::inspect_unsigned(net.chain, &prepared.unsigned)?,
        _ => return Err(WalletError::validation("unsupported native family")),
    };
    let expected_amount = parse_units(&input.amount, net.chain.decimals)?.to_string();
    let actual_to = decoded.get("to").and_then(Value::as_str).ok_or_else(|| {
        WalletError::validation("prepared native transfer has no decoded recipient")
    })?;
    if normalize(net.chain, actual_to)? != input.payee.address
        || decoded.get("amountRaw").and_then(Value::as_str) != Some(expected_amount.as_str())
        || (matches!(net.chain.family, Family::Solana(_))
            && decoded.get("from").and_then(Value::as_str) != Some(input.from.as_str()))
    {
        return Err(WalletError::validation(
            "prepared native transaction differs from the held sender, recipient or amount; nothing signed",
        ));
    }
    // TON signing payloads contain no sender: the signed external-message StateInit and key
    // are checked in inspect_signed, then matched to this pinned sender by send_prepared.
    let verdict = match net.chain.family {
        Family::Solana(_) => chains::solana::check_unsigned(
            net.chain,
            &input.from,
            &prepared.unsigned,
            &ListView::Usable(&checked.list),
        ),
        Family::Ton(_) => chains::ton::check_unsigned(
            net.chain,
            &input.from,
            &prepared.unsigned,
            &ListView::Usable(&checked.list),
        ),
        _ => unreachable!(),
    }
    .and_then(|()| {
        chains::ed25519::check_native(
            &ListView::Usable(&checked.list),
            net.chain,
            &input.from,
            &input.payee.address,
            parse_units(&input.amount, net.chain.decimals)?,
            prepared.max_gas_cost,
        )
    });
    let sufficient = prepared
        .fields
        .get("sufficientFunds")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            WalletError::validation("native preparation omitted its balance assessment")
        })?;
    let mut fields = prepared.fields.clone();
    // Identity always comes from the held input; bytes above already proved recipient/amount.
    fields.extend(base_fields(net.chain, &input));
    fields.insert("amount".into(), json!(input.amount));
    fields.insert("amountRaw".into(), json!(expected_amount));
    fields.insert("symbol".into(), json!(net.chain.symbol));
    fields.insert(
        "whitelist".into(),
        match &verdict {
            Ok(()) => json!({"allowed":true}),
            Err(e) => json!({"allowed":false,"reason":e.message,"code":e.code.as_str()}),
        },
    );
    let window = checked.list.policy.duplicate_window_secs();
    let journal =
        chains::SendJournal::new(&home, net.chain, &input.from).with_duplicate_window(window)?;
    let duplicates = journal.recent_duplicates(
        &send_summary(&fields, net.chain, "transfer"),
        crate::core::time::unix_secs(std::time::SystemTime::now()),
    )?;
    fields.insert("duplicateGuard".into(),json!({"windowSecs":window,"journalTailBytes":1<<20,"advisory":true,"policySource":"password-checked-list"}));
    fields.insert(
        "duplicateSends".into(),
        duplicates
            .iter()
            .map(crate::core::text::inert_json)
            .collect(),
    );
    Ok(NativeTransferReview {
        home,
        net,
        checked,
        input,
        request,
        prepared,
        decoded,
        verdict,
        fields,
        sufficient,
    })
}
/// No signature occurs here. Fingerprints bracket RPC work, and only payees from the held
/// password-checked list can be selected. The caller supplies the selected password-pinned key.
pub async fn review_native_transfer(
    home: &Path,
    net: &Net,
    checked: &CheckedList,
    mut input: NativeTransferInput,
) -> Result<NativeTransferReview> {
    require_native(net.chain)?;
    if input.task_id.is_some() {
        return Err(WalletError::new(
            ErrorCode::CapabilityUnsupported,
            "task attribution and accounting are not available on this family",
        ));
    }
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    if input.payee.fingerprint != checked.fingerprint
        || input.payee.chain != net.chain.key
        || !native_transfer_payees(checked, net.chain)
            .iter()
            .any(|p| p.nickname == input.payee.nickname && p.address == input.payee.address)
    {
        return Err(WalletError::validation(
            "payee is not from this password-checked chain list",
        ));
    }
    input.from = normalize(net.chain, &input.from)?;
    let amount = parse_units(&input.amount, net.chain.decimals)?;
    if amount.is_zero() {
        return Err(WalletError::validation("transfer amount must be positive"));
    }
    let request = TransferRequest {
        from: input.from.clone(),
        to: input.payee.address.clone(),
        amount: input.amount.clone(),
        token: None,
        dry_run: true,
        nonce: None,
        bump: None,
        replace: None,
        max_fee_per_gas: None,
        force: false,
        allow_pending: false,
        journal: Some(chains::SendJournal::new(home, net.chain, &input.from)),
    };
    let prepared = prepare(net, &request).await?;
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    assemble(
        home.into(),
        net.clone(),
        checked.clone(),
        input,
        request,
        prepared,
    )
}
/// Confirm the held nickname, reprepare under the account lock, and require a new review when
/// fee/sufficiency/policy/TON sequence changes. The signed bytes are independently inspected by
/// send_prepared; a fresh request never silently replaces the held payment.
pub async fn send_native_transfer(
    review: NativeTransferReview,
    typed_nickname: &str,
    progress: Option<chains::SendProgress>,
) -> Result<NativeSendOutcome> {
    if typed_nickname != review.payee().nickname() {
        return Err(WalletError::validation(
            "type the payee nickname exactly; nothing signed",
        ));
    }
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    review.verdict.clone()?;
    if !review.sufficient {
        return Err(WalletError::new(
            ErrorCode::InsufficientFunds,
            "insufficient native balance; refresh the review before confirming",
        ));
    }
    let lock = chains::SendLock::acquire(&review.home, review.net.chain, &review.input.from)?;
    let prepared = prepare(&review.net, &review.request).await?;
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let next = assemble(
        review.home.clone(),
        review.net.clone(),
        review.checked.clone(),
        review.input.clone(),
        review.request.clone(),
        prepared,
    )?;
    if !review.matches(&next) {
        return Ok(NativeSendOutcome::ReviewChanged(Box::new(next)));
    }
    next.verdict?;
    if !next.sufficient {
        return Err(WalletError::new(
            ErrorCode::InsufficientFunds,
            "insufficient native balance for amount and estimated fee; nothing signed",
        ));
    }
    let options = SendOptions {
        dry_run: false,
        nonce: None,
        allow_duplicate: next.input.allow_duplicate,
        wait: false,
        wait_timeout: None,
        duplicate_window_secs: Some(next.checked.list.policy.duplicate_window_secs()),
    };
    let fields = base_fields(next.net.chain, &next.input);
    let output = match next.net.chain.family {
        Family::Solana(_) => {
            send_prepared(
                &chains::solana::Solana,
                &next.net,
                &next.home,
                next.input.index,
                &next.input.from,
                next.prepared,
                &options,
                fields,
                Some(lock),
                "transfer",
                progress,
            )
            .await?
        }
        Family::Ton(_) => {
            send_prepared(
                &chains::ton::Ton,
                &next.net,
                &next.home,
                next.input.index,
                &next.input.from,
                next.prepared,
                &options,
                fields,
                Some(lock),
                "transfer",
                progress,
            )
            .await?
        }
        _ => unreachable!(),
    };
    Ok(NativeSendOutcome::Sent(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::whitelist::{Entry, ListFiles, Whitelist};

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap()
    }
    fn setup(
        home: &Path,
    ) -> (
        Net,
        CheckedList,
        NativeTransferInput,
        TransferRequest,
        PreparedTx,
    ) {
        let chain = chains::find("solana").unwrap();
        let from = bs58::encode([1u8; 32]).into_string();
        let to = bs58::encode([2u8; 32]).into_string();
        let mut list = Whitelist::empty(&chains::WHITELIST_RULES);
        list.generation = 1;
        list.entries.push(Entry::new_address(
            "bob",
            &chains::solana::policy_address(&to).unwrap(),
            &["solana"],
        ));
        let checked = CheckedList {
            list,
            source: "whitelist.json",
            fingerprint: ListFiles::read(home).fingerprint(),
        };
        let input = NativeTransferInput {
            task_id: None,
            key_name: "chosen".into(),
            index: 0,
            from: from.clone(),
            payee: native_transfer_payees(&checked, chain).remove(0),
            amount: "0.1".into(),
            allow_duplicate: false,
        };
        let request = TransferRequest {
            from,
            to,
            amount: "0.1".into(),
            token: None,
            dry_run: true,
            nonce: None,
            bump: None,
            replace: None,
            max_fee_per_gas: None,
            force: false,
            allow_pending: false,
            journal: None,
        };
        let unsigned = chains::solana::tx::encode(&chains::solana::tx::Transfer {
            from: [1; 32],
            to: [2; 32],
            blockhash: [3; 32],
            lamports: 100_000_000,
        })
        .unwrap();
        let prepared=PreparedTx { max_gas_cost:U256::from(5000),l1_fee:None,
            unsigned:json!(format!("0x{}",hex::encode(unsigned))),
            fields:json!({"sufficientFunds":true,"balanceRaw":"1000000000","warning":"node fee is an observation"}).as_object().unwrap().clone() };
        (
            Net {
                chain,
                rpc_url: "http://127.0.0.1:9".into(),
            },
            checked,
            input,
            request,
            prepared,
        )
    }
    fn review(home: &Path) -> NativeTransferReview {
        let (net, checked, input, request, prepared) = setup(home);
        assemble(home.into(), net, checked, input, request, prepared).unwrap()
    }
    #[test]
    fn native_review_decodes_bytes_and_never_trusts_the_preparation_labels() {
        let home = home();
        let (net, checked, input, request, mut prepared) = setup(home.path());
        prepared.fields.insert("to".into(), json!("a forged label"));
        prepared
            .fields
            .insert("amountRaw".into(), json!("999999999999"));
        let good = assemble(
            home.path().into(),
            net.clone(),
            checked.clone(),
            input.clone(),
            request.clone(),
            prepared.clone(),
        )
        .unwrap();
        assert_eq!(good.fields()["to"], request.to);
        assert_eq!(good.fields()["amountRaw"], "100000000");
        assert_eq!(good.fields()["warning"], "node fee is an observation");
        for (to, lamports) in [([4; 32], 100_000_000), ([2; 32], 100_000_001)] {
            let bytes = chains::solana::tx::encode(&chains::solana::tx::Transfer {
                from: [1; 32],
                to,
                blockhash: [3; 32],
                lamports,
            })
            .unwrap();
            prepared.unsigned = json!(format!("0x{}", hex::encode(bytes)));
            let error = assemble(
                home.path().into(),
                net.clone(),
                checked.clone(),
                input.clone(),
                request.clone(),
                prepared.clone(),
            )
            .unwrap_err();
            assert!(error.message.contains("differs from the held"));
        }
    }
    #[test]
    fn native_review_requires_balance_assessment_and_reconfirmation_for_fee_or_funds_change() {
        let home = home();
        let original = review(home.path());
        let (net, checked, input, request, mut prepared) = setup(home.path());
        prepared.fields.remove("sufficientFunds");
        assert!(
            assemble(
                home.path().into(),
                net.clone(),
                checked.clone(),
                input.clone(),
                request.clone(),
                prepared.clone()
            )
            .is_err()
        );
        prepared
            .fields
            .insert("sufficientFunds".into(), json!(true));
        prepared.max_gas_cost = U256::from(5001);
        let dearer = assemble(
            home.path().into(),
            net.clone(),
            checked.clone(),
            input.clone(),
            request.clone(),
            prepared.clone(),
        )
        .unwrap();
        assert!(!original.matches(&dearer));
        prepared.max_gas_cost = U256::from(5000);
        prepared
            .fields
            .insert("sufficientFunds".into(), json!(false));
        let poor = assemble(
            home.path().into(),
            net.clone(),
            checked.clone(),
            input.clone(),
            request.clone(),
            prepared.clone(),
        )
        .unwrap();
        assert!(!original.matches(&poor));
        prepared
            .fields
            .insert("sufficientFunds".into(), json!(true));
        let bytes = chains::solana::tx::encode(&chains::solana::tx::Transfer {
            from: [1; 32],
            to: [2; 32],
            blockhash: [9; 32],
            lamports: 100_000_000,
        })
        .unwrap();
        prepared.unsigned = json!(format!("0x{}", hex::encode(bytes)));
        let refreshed =
            assemble(home.path().into(), net, checked, input, request, prepared).unwrap();
        assert!(
            original.matches(&refreshed),
            "a blockhash refresh preserves payment and fee"
        );
    }
    #[tokio::test]
    async fn bad_nickname_changed_list_and_insufficient_balance_refuse_without_rpc_or_signature() {
        let home = home();
        // Closed RPC, no daemon socket: each expected error proves this gate precedes either.
        let error = send_native_transfer(review(home.path()), "someone else", None)
            .await
            .unwrap_err();
        assert!(error.message.contains("nickname exactly"));
        let mut poor = review(home.path());
        poor.sufficient = false;
        assert_eq!(
            send_native_transfer(poor, "bob", None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InsufficientFunds
        );
        let mut denied = review(home.path());
        denied.verdict = Err(WalletError::validation("policy blocked this payment"));
        assert_eq!(
            send_native_transfer(denied, "bob", None)
                .await
                .unwrap_err()
                .message,
            "policy blocked this payment"
        );
        let held = review(home.path());
        std::fs::write(home.path().join("whitelist.json"), b"changed").unwrap();
        let error = send_native_transfer(held, "bob", None).await.unwrap_err();
        assert!(error.message.contains("changed"), "{}", error.message);
        assert!(!home.path().join("d.sock").exists());
    }
    #[test]
    fn native_payees_are_exact_chain_and_never_wildcards_or_contracts() {
        let home = home();
        let (net, mut checked, _, _, _) = setup(home.path());
        let original = checked.list.entries[0].clone();
        let mut wrong = original.clone();
        wrong.nickname = "wrong".into();
        wrong.chains = vec!["*".into()];
        checked.list.entries.push(wrong);
        let mut wrong = original.clone();
        wrong.nickname = "mixed".into();
        wrong.chains = vec!["solana".into(), "base".into()];
        checked.list.entries.push(wrong);
        let mut wrong = original;
        wrong.nickname = "contract".into();
        wrong.kind = EntryKind::Contract;
        checked.list.entries.push(wrong);
        let payees = native_transfer_payees(&checked, net.chain);
        assert_eq!(payees.len(), 1);
        assert_eq!(payees[0].nickname(), "bob");
        assert!(native_transfer_payees(&checked, chains::find("ton").unwrap()).is_empty());
    }
}

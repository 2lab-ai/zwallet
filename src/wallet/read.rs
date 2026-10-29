//! Read-only wallet operations shared by CLI rendering and the TUI.

use super::{Trust, address, net};
use crate::chains::evm::{Evm, rpc};
use crate::chains::{self, CHAINS, Chain, ChainFamily, Family, Net};
use crate::commands::Daemon;
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::store;
use crate::core::text::ascii_inert;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(8);
pub type NetworkBalances = Result<(chains::Balance, Vec<chains::TokenBalance>)>;
#[derive(Clone, Debug)]
pub struct NetworkRead {
    pub native: chains::Balance,
    pub tokens: Vec<chains::TokenBalance>,
    pub activated: Option<bool>,
    pub warning: Option<String>,
}
#[derive(Clone, Debug)]
pub struct AssetRead {
    pub balance: chains::Balance,
    pub activated: Option<bool>,
    pub token: Option<String>,
    pub symbol_source: Option<&'static str>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChainBalances {
    pub chain: &'static Chain,
    pub key: String,
    pub address: String,
    pub trust: Trust,
    pub native: chains::Balance,
    pub tokens: Vec<chains::TokenBalance>,
    pub activated: Option<bool>,
    pub warning: Option<String>,
}
#[derive(Clone, Debug)]
pub struct NetworkBalance {
    pub chain: &'static Chain,
    pub address: Option<String>,
    pub balances: Result<NetworkRead>,
}
#[derive(Clone, Debug)]
pub struct EveryBalance {
    pub key: String,
    pub address: String,
    pub trust: Trust,
    pub networks: Vec<NetworkBalance>,
}
#[derive(Clone, Debug)]
pub struct AddressRow {
    pub error: Option<WalletError>,
    pub chain: &'static Chain,
    pub address: String,
}
#[derive(Clone, Debug)]
pub struct Addresses {
    pub key: String,
    pub trust: Trust,
    pub rows: Vec<AddressRow>,
}

pub fn net_with(chain: &'static Chain, url: &str) -> Net {
    Net {
        chain,
        rpc_url: chain.resolve_rpc(Some(url), None),
    }
}
/// The endpoint must answer for this chain before the session adopts it.
pub async fn validate_rpc(net: &Net) -> Result<()> {
    tokio::time::timeout(NETWORK_TIMEOUT, async {
        match net.chain.family {
            Family::Evm(_) => rpc::connect(net).await.map(|_| ()),
            Family::Tron(_) => chains::tron::rpc::connect(net).await.map(|_| ()),
        }
    })
    .await
    .map_err(|_| WalletError::rpc("RPC chain check timed out"))?
}
pub async fn read_network(net: Net, owner: String) -> NetworkBalances {
    read_network_details(net, owner)
        .await
        .map(|r| (r.native, r.tokens))
}
pub async fn read_network_details(net: Net, owner: String) -> Result<NetworkRead> {
    match net.chain.family {
        Family::Evm(_) => Evm
            .wallet_balances(&net, &owner, NETWORK_TIMEOUT)
            .await
            .map(|(native, tokens)| NetworkRead {
                native,
                tokens,
                activated: None,
                warning: None,
            }),
        Family::Tron(_) => chains::tron::wallet_balances(&net, &owner, NETWORK_TIMEOUT)
            .await
            .map(|r| NetworkRead {
                native: r.native,
                tokens: r.tokens,
                activated: Some(r.activated),
                warning: r.warning,
            }),
    }
}

pub async fn balances(home: &Path, key: &str, net: Net) -> Result<ChainBalances> {
    let (owner, trust) = address(home, key, net.chain).await?;
    let chain = net.chain;
    let NetworkRead {
        native,
        tokens,
        activated,
        warning,
    } = read_network_details(net, owner.clone()).await?;
    Ok(ChainBalances {
        chain,
        key: key.to_string(),
        address: owner,
        trust,
        native,
        tokens,
        activated,
        warning,
    })
}
/// The one-asset CLI path retains its response and does not add token RPCs.
pub async fn balance(
    home: &Path,
    key: &str,
    net: &Net,
    token: Option<&str>,
) -> Result<(String, Trust, chains::Balance)> {
    let (owner, trust, read) = balance_detail(home, key, net, token).await?;
    Ok((owner, trust, read.balance))
}
pub async fn balance_detail(
    home: &Path,
    key: &str,
    net: &Net,
    token: Option<&str>,
) -> Result<(String, Trust, AssetRead)> {
    let (owner, trust) = address(home, key, net.chain).await?;
    let read = match net.chain.family {
        Family::Evm(_) => AssetRead {
            balance: match token {
                Some(token) => Evm.token_balance(net, &owner, token).await?,
                None => Evm.native_balance(net, &owner).await?,
            },
            activated: None,
            token: token.map(|t| Evm.normalize_address(t)).transpose()?,
            symbol_source: None,
            warning: None,
        },
        Family::Tron(_) => {
            let r = chains::tron::balance(net, &owner, token).await?;
            AssetRead {
                balance: r.balance,
                activated: Some(r.activated),
                token: r.token.as_ref().map(|t| t.address.clone()),
                symbol_source: r.token.map(|t| {
                    if t.from_registry {
                        "registry"
                    } else {
                        "contract"
                    }
                }),
                warning: r.warning,
            }
        }
    };
    Ok((owner, trust, read))
}

pub async fn addresses(home: &Path, key_name: &str) -> Result<Addresses> {
    let file = store::read_wallet_file(home)?;
    let key = file.key(Some(key_name))?;
    let mut daemon = Daemon::new(home);
    let mut derived = BTreeMap::new();
    let mut rows = Vec::new();
    for chain in CHAINS {
        let family = chain.family.name();
        if !derived.contains_key(family) {
            derived.insert(family, daemon.derive(key.index, chain).await?);
        }
        match daemon.checked(key, chain, derived[family].as_ref()).await {
            Ok(address) => rows.push(AddressRow {
                chain,
                address,
                error: None,
            }),
            Err(e)
                if matches!(chain.family, Family::Tron(_)) && e.code == ErrorCode::WalletLocked =>
            {
                rows.push(AddressRow {
                    chain,
                    address: String::new(),
                    error: Some(e),
                })
            }
            Err(e) => return Err(e),
        }
    }
    Ok(Addresses {
        key: key.name.clone(),
        trust: daemon.trust(),
        rows,
    })
}
/// Resolve all addresses before RPC: a malformed address or contradicted cache aborts the whole
/// read. A daemon transport failure becomes a per-family error, as the CLI has always done.
pub async fn every_balance(home: &Path, key_name: &str) -> Result<EveryBalance> {
    every_balance_with(home, key_name, CHAINS.iter().map(net).collect()).await
}
/// The TUI passes endpoints captured at startup, including session-only overrides.
pub async fn every_balance_with(
    home: &Path,
    key_name: &str,
    mut nets: Vec<Net>,
) -> Result<EveryBalance> {
    nets.sort_by_key(|n| n.chain.is_testnet());
    let file = store::read_wallet_file(home)?;
    let key = file.key(Some(key_name))?;
    let mut daemon = Daemon::new(home);
    let mut derived = BTreeMap::new();
    let mut owners = Vec::with_capacity(nets.len());
    for net in &nets {
        let chain = net.chain;
        let family = chain.family.name();
        if !derived.contains_key(family) {
            derived.insert(family, daemon.fetch(key.index, chain).await?);
        }
        owners.push(match &derived[family] {
            Ok(Some(value)) => {
                daemon.check_cache(key, chain, value)?;
                Ok(value.clone())
            }
            Ok(None) => daemon.cache_only(key, chain).await,
            Err(error) => Err(error.clone()),
        });
    }
    let address = owners
        .iter()
        .find_map(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let trust = daemon.trust();
    let reads: Vec<_> = nets
        .into_iter()
        .zip(owners)
        .map(|(net, owner)| {
            let chain = net.chain;
            let read = owner.map(|owner| {
                let task = tokio::spawn(read_network_details(net, owner.clone()));
                (owner, task)
            });
            (chain, read)
        })
        .collect();
    let mut networks = Vec::with_capacity(reads.len());
    for (chain, read) in reads {
        let (address, balances) = match read {
            Ok((owner, read)) => (
                Some(owner),
                read.await.unwrap_or_else(|_| {
                    Err(WalletError::rpc("the balance read stopped unexpectedly"))
                }),
            ),
            Err(error) => (None, Err(error)),
        };
        networks.push(NetworkBalance {
            chain,
            address,
            balances,
        });
    }
    Ok(EveryBalance {
        key: key.name.clone(),
        address,
        trust,
        networks,
    })
}
/// One inert line of at most 72 characters, including its truncation marker.
pub fn brief(message: &str) -> String {
    ascii_inert(message.lines().next().unwrap_or_default(), 72)
}

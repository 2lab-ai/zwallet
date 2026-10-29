//! Display addresses are scoped to a key and family. Primary-EVM metadata never labels Tron.
use super::*;

pub(super) fn validate_family_address(chain: &Chain, address: &str) -> Result<()> {
    match chain.family {
        crate::chains::Family::Evm(_) => crate::chains::evm::parse_address(address).map(|_| ()),
        crate::chains::Family::Tron(_) => crate::chains::tron::address::parse(address).map(|_| ()),
    }
}
impl App {
    /// Only callers whose generation/key/chain envelope has been checked may record a read.
    pub(super) fn observe_family_address(
        &mut self,
        key: &str,
        chain: &Chain,
        address: &str,
        trust: Trust,
    ) -> Result<()> {
        self.check_family_pin(key, chain, address)?;
        self.nav
            .family_reads
            .insert((key.into(), chain.family.name()), (address.into(), trust));
        Ok(())
    }
    /// An address suitable for a row/header on the selected family. Cached primary key rows
    /// are EVM-only; a Tron address must have its own scoped observation or password pin.
    pub fn displayed_address(&self, key: &str) -> Option<(Outside, TrustLine)> {
        let read = self
            .nav
            .family_reads
            .get(&(key.into(), self.chain.family.name()));
        if let Some(pin) = self.family_pin(key, self.chain) {
            validate_family_address(self.chain, pin).ok()?;
            let trust = match read {
                Some((_, Trust::Unverified(Some(w)))) => TrustLine::Impostor(Outside::warning(w)),
                _ => TrustLine::Password,
            };
            return Some((Outside::address(pin), trust));
        }
        if let Some((address, trust)) = read {
            validate_family_address(self.chain, address).ok()?;
            return Some((Outside::address(address), trust_line(trust.clone())));
        }
        if matches!(self.chain.family, crate::chains::Family::Evm(_))
            && let Pane::Ready(keys) = &self.keys
            && let Some(address) = keys
                .rows
                .iter()
                .find(|r| r.key == key)
                .and_then(|r| r.address.as_ref())
        {
            validate_family_address(self.chain, address.text()).ok()?;
            return Some((
                address.clone(),
                keys.warning
                    .clone()
                    .map_or(TrustLine::NotVerified, TrustLine::Impostor),
            ));
        }
        None
    }
    pub fn display_bar(&self, bar: &Bar) -> Bar {
        let mut shown = bar.clone();
        shown.address = None;
        shown.trust = match &bar.trust {
            Some(TrustLine::Impostor(w)) => Some(TrustLine::Impostor(w.clone())),
            _ => None,
        };
        let key = self
            .open_key
            .as_deref()
            .or_else(|| self.nav.session.as_ref().and_then(|s| s.active.as_deref()));
        if let Some(key) = key {
            shown.key = Some(Outside::key_name(key));
            if let Some((address, trust)) = self.displayed_address(key) {
                shown.address = Some(Outside::short_address(address.text()));
                shown.trust = Some(match &bar.trust {
                    Some(TrustLine::Impostor(w)) => TrustLine::Impostor(w.clone()),
                    _ => trust,
                });
            }
        }
        shown
    }
}

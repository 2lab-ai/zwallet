//! The wallet's single BIP39 mnemonic. Every chain family derives its own key from it.
//!
//! Error messages are fixed strings: BIP39 errors can quote the offending word, and a mnemonic
//! word is a secret. `Seed` is deliberately not `Debug`/`Clone`/`Serialize`.
//!
//! The phrase itself lives in a `Zeroizing<String>`. Accepted residue: copies made inside
//! coins-bip39, whose API offers no way to wipe them (`Mnemonic` and `Entropy` have private,
//! non-`Zeroize` fields), are freed without wiping:
//! - `Seed::generate`: the `Entropy` copy held by `Mnemonic`, and the `BitVec` of entropy bits and
//!   the `Vec<&str>` of word pointers that `Mnemonic::to_phrase` builds.
//! - `Seed::from_phrase` (every decrypt): `Mnemonic::new_from_phrase` builds a `BitVec` and an
//!   `Entropy` copy, then a full `to_phrase()` `String` for its checksum comparison; a checksum
//!   mismatch puts another copy of the phrase into `MnemonicError::InvalidPhrase`.
//! - `chains::evm::signer_at` (every derive and sign in the daemon): alloy's `MnemonicBuilder`
//!   wipes its own phrase copy on drop (alloy-signer-local's `zeroize` feature is on), but the
//!   coins-bip39 calls under it repeat the `new_from_phrase` copies above, and `to_seed` returns
//!   the 64-byte BIP39 seed by value next to one more `to_phrase()` `String`.
//!
//! These sit in short-lived processes (`create`, `backup`, `unlock`, and the daemon, which exits
//! on lock or expiry), all of which disable core dumps; what remains is swap and a debugger
//! attached by the same user, which the threat model already concedes.

use crate::core::error::{Result, WalletError};
use alloy::signers::local::coins_bip39::{English, Entropy, Mnemonic};
use rand::RngCore;
use rand::rngs::OsRng;
use zeroize::Zeroizing;

pub struct Seed {
    phrase: Zeroizing<String>,
}

/// BIP39 word counts; the default is the maximum (256-bit entropy).
pub const VALID_WORD_COUNTS: [usize; 5] = [12, 15, 18, 21, 24];
pub const DEFAULT_WORD_COUNT: usize = 24;

fn take<const N: usize>(buf: &[u8; 32]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&buf[..N]);
    out
}

impl Seed {
    /// A fresh English mnemonic of `words` words (12/15/18/21/24) from OS randomness, no BIP39
    /// passphrase.
    pub fn generate(words: usize) -> Result<Self> {
        let mut buf = Zeroizing::new([0u8; 32]);
        OsRng
            .try_fill_bytes(buf.as_mut())
            .map_err(|_| WalletError::validation("could not read OS randomness"))?;
        let entropy = match words {
            12 => Entropy::from(take::<16>(&buf)),
            15 => Entropy::from(take::<20>(&buf)),
            18 => Entropy::from(take::<24>(&buf)),
            21 => Entropy::from(take::<28>(&buf)),
            24 => Entropy::from(*buf),
            _ => {
                return Err(WalletError::validation(format!(
                    "--words must be one of {VALID_WORD_COUNTS:?}, got {words}"
                )));
            }
        };
        let mnemonic = Mnemonic::<English>::new_from_entropy(entropy);
        let seed = Self::from_phrase(Zeroizing::new(mnemonic.to_phrase()))?;
        if seed.word_count() != words {
            return Err(WalletError::validation(
                "generated mnemonic failed self-check",
            ));
        }
        Ok(seed)
    }

    /// Wraps a phrase after checking it is a valid English BIP39 mnemonic.
    pub fn from_phrase(phrase: Zeroizing<String>) -> Result<Self> {
        if Mnemonic::<English>::new_from_phrase(&phrase).is_err() {
            return Err(WalletError::validation(
                "mnemonic is not a valid BIP39 English phrase",
            ));
        }
        Ok(Self { phrase })
    }

    /// The phrase itself. Callers must never log, print, or return it, except `wallet backup`.
    pub fn phrase(&self) -> &str {
        &self.phrase
    }

    pub fn word_count(&self) -> usize {
        self.phrase.split(' ').count()
    }
}

#[cfg(test)]
pub(crate) const TEST_MNEMONIC: &str =
    "test test test test test test test test test test test junk";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_seeds_have_requested_word_count_and_are_distinct() {
        for words in VALID_WORD_COUNTS {
            assert_eq!(Seed::generate(words).unwrap().word_count(), words);
        }
        let a = Seed::generate(DEFAULT_WORD_COUNT).unwrap();
        let b = Seed::generate(DEFAULT_WORD_COUNT).unwrap();
        assert_eq!(a.word_count(), 24);
        assert_ne!(a.phrase(), b.phrase());
        assert!(Seed::generate(13).is_err());
    }

    #[test]
    fn invalid_phrase_error_does_not_echo_words() {
        let err = Seed::from_phrase(Zeroizing::new("zebra secretword alpha".into()))
            .err()
            .unwrap();
        assert!(!err.message.contains("secretword"));
        assert!(!err.message.contains("zebra"));
    }
}

//! Secret input has one fixed allocation, no formatting or clone implementation. The input
//! thread owns it until submission; UI messages carry only its character count.
use crate::core::password::MAX_SECRET;
use zeroize::{Zeroize, Zeroizing};

pub struct PasswordBuf {
    bytes: Zeroizing<Box<[u8]>>,
    len: usize,
}

impl zeroize::ZeroizeOnDrop for PasswordBuf {}

impl Default for PasswordBuf {
    fn default() -> Self {
        Self {
            bytes: Zeroizing::new(vec![0; MAX_SECRET].into_boxed_slice()),
            len: 0,
        }
    }
}

impl AsRef<str> for PasswordBuf {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PasswordBuf {
    pub fn as_str(&self) -> &str {
        // Only UTF-8 chars or strings are appended; removal always uses a char boundary.
        std::str::from_utf8(&self.bytes[..self.len]).expect("password buffer is UTF-8")
    }

    pub fn characters(&self) -> usize {
        self.as_str().chars().count()
    }

    /// Refuse a whole overlong append (never silently sign in with a truncated password).
    pub fn append(&mut self, text: &str) -> bool {
        if text.contains(['\n', '\r', '\0']) || self.len + text.len() >= MAX_SECRET {
            return false;
        }
        let end = self.len + text.len();
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        true
    }

    pub fn push(&mut self, c: char) -> bool {
        let mut encoded = Zeroizing::new([0; 4]);
        self.append(c.encode_utf8(&mut *encoded))
    }

    /// Crossterm owns a paste String; wipe that copy immediately, even on rejection.
    pub fn paste(&mut self, text: &mut String) -> bool {
        let accepted = self.append(text);
        text.zeroize();
        accepted
    }

    pub fn backspace(&mut self) {
        if let Some((start, _)) = self.as_str().char_indices().next_back() {
            self.bytes[start..self.len].zeroize();
            self.len = start;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paste_wipes_its_copy_and_input_has_one_bound() {
        let mut p = PasswordBuf::default();
        let mut paste = String::from("pw-XYZ-0123456789");
        let ptr = paste.as_ptr();
        let len = paste.len();
        assert!(p.paste(&mut paste));
        assert!(paste.is_empty());
        // Zeroize clears without freeing/reallocating String's allocation. These initialized
        // bytes remain valid until `paste` drops; inspect without ever formatting them.
        assert!(
            unsafe { std::slice::from_raw_parts(ptr, len) }
                .iter()
                .all(|b| *b == 0)
        );
        assert_eq!(p.characters(), 17);
        assert!(p.push('é'));
        p.backspace();
        assert_eq!(p.characters(), 17);
        assert!(!p.append(&"x".repeat(MAX_SECRET)));
        assert!(!p.append("\n"));
        assert_eq!(p.characters(), 17);
    }
    #[test]
    fn a_password_accepted_by_the_cli_can_contain_a_tab() {
        let mut pasted = String::from("abcdefgh\txyz");
        crate::core::store::validate_new_password(&pasted).unwrap();
        let mut password = PasswordBuf::default();
        assert!(password.paste(&mut pasted));
        assert!(pasted.is_empty());
        assert!(password.as_ref() == "abcdefgh\txyz");
        assert!(password.push('\t'));
        assert_eq!(password.characters(), 13);
        assert!(!password.append("\r") && !password.append("\0"));
    }
}

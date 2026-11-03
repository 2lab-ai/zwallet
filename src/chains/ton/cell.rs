//! Bounded ordinary level-zero cells for the exact V4R2 grammar. No exotic cells, caches,
//! multiple roots or cyclic references. This is not a general-purpose TON deserializer.
use crate::core::error::{Result, WalletError};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

pub(super) const MAX_BOC: usize = 16 * 1024;
fn bad() -> WalletError {
    WalletError::validation("invalid or unsupported TON cell encoding")
}
#[derive(Clone, Debug)]
pub(super) struct Cell {
    pub data: Vec<u8>,
    pub bits: usize,
    pub refs: Vec<Arc<Cell>>,
    hash: OnceLock<[u8; 32]>,
    depth: u16,
}
impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.hash() == other.hash()
    }
}
impl Eq for Cell {}
impl Cell {
    pub fn hash(&self) -> [u8; 32] {
        *self.hash.get_or_init(|| {
            let mut v = vec![
                self.refs.len() as u8,
                (self.bits / 8 + self.bits.div_ceil(8)) as u8,
            ];
            v.extend(self.padded());
            for r in &self.refs {
                v.extend(r.depth().to_be_bytes());
            }
            for r in &self.refs {
                v.extend(r.hash());
            }
            Sha256::digest(v).into()
        })
    }
    fn depth(&self) -> u16 {
        self.depth
    }
    fn padded(&self) -> Vec<u8> {
        let mut d = self.data.clone();
        if !self.bits.is_multiple_of(8) {
            *d.last_mut().unwrap() |= 1 << (7 - self.bits % 8);
        }
        d
    }
    pub fn reader(&self) -> Reader<'_> {
        Reader {
            cell: self,
            pos: 0,
            reference: 0,
        }
    }
}
#[derive(Default)]
pub(super) struct Builder {
    data: Vec<u8>,
    bits: usize,
    refs: Vec<Arc<Cell>>,
}
impl Builder {
    pub fn uint(&mut self, value: u64, bits: usize) -> &mut Self {
        assert!(bits <= 64);
        for i in (0..bits).rev() {
            if self.bits.is_multiple_of(8) {
                self.data.push(0);
            }
            if (value >> i) & 1 != 0 {
                self.data[self.bits / 8] |= 1 << (7 - self.bits % 8);
            }
            self.bits += 1;
        }
        self
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        for v in b {
            self.uint(*v as u64, 8);
        }
        self
    }
    pub fn coins(&mut self, value: u64) -> &mut Self {
        let n = (64 - value.leading_zeros() as usize).div_ceil(8);
        self.uint(n as u64, 4);
        if n > 0 {
            self.uint(value, n * 8);
        }
        self
    }
    pub fn reference(&mut self, c: Arc<Cell>) -> &mut Self {
        self.refs.push(c);
        self
    }
    pub fn append(&mut self, c: &Cell) -> &mut Self {
        for i in 0..c.bits {
            self.uint(((c.data[i / 8] >> (7 - i % 8)) & 1) as u64, 1);
        }
        self.refs.extend(c.refs.clone());
        self
    }
    pub fn end(self) -> Arc<Cell> {
        assert!(self.bits <= 1023 && self.refs.len() <= 4);
        {
            let depth = self.refs.iter().map(|c| c.depth() + 1).max().unwrap_or(0);
            Arc::new(Cell {
                data: self.data,
                bits: self.bits,
                refs: self.refs,
                hash: OnceLock::new(),
                depth,
            })
        }
    }
}
pub(super) struct Reader<'a> {
    cell: &'a Cell,
    pos: usize,
    reference: usize,
}
impl Reader<'_> {
    pub fn uint(&mut self, n: usize) -> Result<u64> {
        if n > 64 || self.pos + n > self.cell.bits {
            return Err(bad());
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | ((self.cell.data[self.pos / 8] >> (7 - self.pos % 8)) & 1) as u64;
            self.pos += 1;
        }
        Ok(v)
    }
    pub fn expect(&mut self, n: usize, v: u64) -> Result<()> {
        if self.uint(n)? != v {
            return Err(bad());
        }
        Ok(())
    }
    pub fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut v = [0; N];
        for b in &mut v {
            *b = self.uint(8)? as u8;
        }
        Ok(v)
    }
    pub fn coins(&mut self) -> Result<u64> {
        let n = self.uint(4)? as usize;
        if n > 8 {
            return Err(bad());
        }
        let v = self.uint(n * 8)?;
        if n > 0 && (v == 0 || (64 - v.leading_zeros() as usize).div_ceil(8) != n) {
            return Err(bad());
        }
        Ok(v)
    }
    pub fn reference(&mut self) -> Result<Arc<Cell>> {
        let r = self.cell.refs.get(self.reference).ok_or_else(bad)?.clone();
        self.reference += 1;
        Ok(r)
    }
    pub fn finish(self) -> Result<()> {
        if self.pos != self.cell.bits || self.reference != self.cell.refs.len() {
            return Err(bad());
        }
        Ok(())
    }
}
fn integer(bytes: &[u8], at: &mut usize, n: usize) -> Result<usize> {
    if n == 0 || n > 4 || *at + n > bytes.len() {
        return Err(bad());
    }
    let mut v = 0;
    for b in &bytes[*at..*at + n] {
        v = (v << 8) | *b as usize;
    }
    *at += n;
    Ok(v)
}
fn crc32c(bytes: &[u8]) -> [u8; 4] {
    let mut c = !0u32;
    for b in bytes {
        c ^= *b as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ if c & 1 != 0 { 0x82f63b78 } else { 0 };
        }
    }
    (!c).to_le_bytes()
}
pub(super) fn decode(bytes: &[u8]) -> Result<Arc<Cell>> {
    if bytes.len() < 11 || bytes.len() > MAX_BOC || bytes[..4] != [0xb5, 0xee, 0x9c, 0x72] {
        return Err(bad());
    }
    let flags = bytes[4];
    if flags & 0x38 != 0 {
        return Err(bad());
    }
    let sz = (flags & 7) as usize;
    let off = bytes[5] as usize;
    let mut at = 6;
    let count = integer(bytes, &mut at, sz)?;
    let roots = integer(bytes, &mut at, sz)?;
    let absent = integer(bytes, &mut at, sz)?;
    let total = integer(bytes, &mut at, off)?;
    if count == 0 || count > 128 || roots != 1 || absent != 0 || integer(bytes, &mut at, sz)? != 0 {
        return Err(bad());
    }
    if flags & 0x80 != 0 {
        for _ in 0..count {
            integer(bytes, &mut at, off)?;
        }
    }
    let end = at.checked_add(total).ok_or_else(bad)?;
    let crc = if flags & 0x40 != 0 { 4 } else { 0 };
    if end + crc != bytes.len() {
        return Err(bad());
    }
    if crc != 0 && bytes[end..] != crc32c(&bytes[..end]) {
        return Err(bad());
    }
    let mut rows = Vec::new();
    for i in 0..count {
        if at + 2 > end {
            return Err(bad());
        }
        let d1 = bytes[at];
        let d2 = bytes[at + 1];
        at += 2;
        if d1 > 4 {
            return Err(bad());
        }
        let len = (d2 as usize).div_ceil(2);
        if at + len > end {
            return Err(bad());
        }
        let mut data = bytes[at..at + len].to_vec();
        at += len;
        let bits = if d2 & 1 != 0 {
            let last = *data.last().ok_or_else(bad)?;
            if last == 0 {
                return Err(bad());
            }
            let padding = last.trailing_zeros() as usize + 1;
            if padding == 8 {
                return Err(bad());
            }
            *data.last_mut().unwrap() &= !(1 << (padding - 1));
            len * 8 - padding
        } else {
            len * 8
        };
        if bits > 1023 {
            return Err(bad());
        }
        let mut refs = Vec::new();
        for _ in 0..d1 {
            let n = integer(bytes, &mut at, sz)?;
            if n <= i || n >= count || at > end {
                return Err(bad());
            }
            refs.push(n);
        }
        rows.push((data, bits, refs));
    }
    if at != end {
        return Err(bad());
    }
    let mut built: Vec<Option<Arc<Cell>>> = vec![None; count];
    let mut depths = vec![0u16; count];
    for (i, (data, bits, refs)) in rows.into_iter().enumerate().rev() {
        let depth = refs.iter().map(|r| depths[*r] + 1).max().unwrap_or(0);
        if depth > 32 {
            return Err(bad());
        }
        depths[i] = depth;
        built[i] = Some(Arc::new(Cell {
            data,
            bits,
            hash: OnceLock::new(),
            depth,
            refs: refs
                .into_iter()
                .map(|r| built[r].as_ref().unwrap().clone())
                .collect(),
        }));
    }
    Ok(built[0].take().unwrap())
}
fn put(v: usize, n: usize, out: &mut Vec<u8>) {
    for i in (0..n).rev() {
        out.push((v >> (i * 8)) as u8);
    }
}
pub(super) fn encode(root: &Arc<Cell>) -> Vec<u8> {
    fn visit(c: &Arc<Cell>, seen: &mut BTreeMap<[u8; 32], bool>, all: &mut Vec<Arc<Cell>>) {
        if seen.insert(c.hash(), true).is_some() {
            return;
        }
        for r in &c.refs {
            visit(r, seen, all);
        }
        all.push(c.clone());
    }
    let mut all = Vec::new();
    visit(root, &mut BTreeMap::new(), &mut all);
    all.reverse();
    let ids: BTreeMap<_, _> = all.iter().enumerate().map(|(i, c)| (c.hash(), i)).collect();
    let mut cells = Vec::new();
    for c in &all {
        cells.push(c.refs.len() as u8);
        cells.push((c.bits / 8 + c.bits.div_ceil(8)) as u8);
        cells.extend(c.padded());
        for r in &c.refs {
            cells.push(ids[&r.hash()] as u8);
        }
    }
    let offset = if cells.len() > 255 { 2 } else { 1 };
    let mut out = vec![
        0xb5,
        0xee,
        0x9c,
        0x72,
        1,
        offset as u8,
        all.len() as u8,
        1,
        0,
    ];
    put(cells.len(), offset, &mut out);
    out.push(0);
    out.extend(cells);
    out
}

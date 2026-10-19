//! MIX archives: a flat list of files addressed by a hash of their name.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use blowfish::cipher::{BlockCipherDecrypt, KeyInit};
use blowfish::Blowfish;
use memmap2::Mmap;
use num_bigint::BigUint;

const FLAG_ENCRYPTED: u32 = 0x0002_0000;
const KEY_SOURCE_LEN: usize = 80;
const PUBLIC_KEY: &str = "AihRvNoIbTn85FZRYNZRcT+i6KpU+maCsEqr3Q5q+LDB5tH7Tz2qQ38V";

type Backing = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// The id a file name is stored under.
pub fn mix_id(name: &str) -> u32 {
    let mut bytes: Vec<u8> = name.bytes().map(|b| b.to_ascii_uppercase()).collect();
    let len = bytes.len();
    let rem = len & 3;
    if rem != 0 {
        let base = len - rem;
        bytes.push(rem as u8);
        for _ in 0..3 - rem {
            bytes.push(bytes[base]);
        }
    }
    crc32fast::hash(&bytes)
}

#[derive(Clone)]
pub struct Mix {
    backing: Backing,
    entries: HashMap<u32, Range<usize>>,
}

impl Mix {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: the game data files are not modified while the program runs.
        let map = unsafe { Mmap::map(&file)? };
        let len = map.len();
        Self::parse(Arc::new(map), 0..len)
    }

    fn parse(backing: Backing, range: Range<usize>) -> io::Result<Self> {
        let data = &(*backing).as_ref()[range.clone()];
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
        if data.len() < 10 {
            return Err(bad("mix too short"));
        }
        let flags = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let has_flags = flags & 0xFFFF == 0;

        let (index, body_start): (Vec<u8>, usize) = if has_flags && flags & FLAG_ENCRYPTED != 0 {
            let key_end = 4 + KEY_SOURCE_LEN;
            let key = blowfish_key(data.get(4..key_end).ok_or_else(|| bad("truncated key"))?);
            let cipher: Blowfish = Blowfish::new_from_slice(&key).map_err(|_| bad("bad key"))?;
            let mut first = data
                .get(key_end..key_end + 8)
                .ok_or_else(|| bad("truncated header"))?
                .to_vec();
            cipher.decrypt_block((&mut first[..]).try_into().unwrap());
            let count = u16::from_le_bytes([first[0], first[1]]) as usize;
            let header_len = (6 + count * 12).div_ceil(8) * 8;
            let mut header = data
                .get(key_end..key_end + header_len)
                .ok_or_else(|| bad("truncated index"))?
                .to_vec();
            for block in header.as_chunks_mut::<8>().0 {
                cipher.decrypt_block(block.into());
            }
            (header[..6 + count * 12].to_vec(), key_end + header_len)
        } else {
            let start = if has_flags { 4 } else { 0 };
            let count = u16::from_le_bytes([data[start], data[start + 1]]) as usize;
            let end = start + 6 + count * 12;
            (
                data.get(start..end).ok_or_else(|| bad("truncated index"))?.to_vec(),
                end,
            )
        };

        let mut entries = HashMap::new();
        for entry in index[6..].as_chunks::<12>().0 {
            let id = u32::from_le_bytes(entry[0..4].try_into().unwrap());
            let offset = u32::from_le_bytes(entry[4..8].try_into().unwrap()) as usize;
            let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let start = range.start + body_start + offset;
            if start + size > range.end {
                return Err(bad("entry outside archive"));
            }
            entries.insert(id, start..start + size);
        }
        Ok(Self { backing, entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }

    pub fn get_id(&self, id: u32) -> Option<&[u8]> {
        let range = self.entries.get(&id)?;
        Some(&(*self.backing).as_ref()[range.clone()])
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.get_id(mix_id(name))
    }

    /// Opens an archive stored inside this one, sharing the same backing file.
    pub fn nested(&self, name: &str) -> Option<io::Result<Mix>> {
        let range = self.entries.get(&mix_id(name))?;
        Some(Self::parse(self.backing.clone(), range.clone()))
    }
}

/// Recovers the Blowfish key for an encrypted header from the 80-byte block that
/// precedes it, using Westwood's public RSA key.
fn blowfish_key(source: &[u8]) -> Vec<u8> {
    let der = base64::engine::general_purpose::STANDARD
        .decode(PUBLIC_KEY)
        .expect("public key is valid base64");
    let modulus = BigUint::from_bytes_be(&der[2..]);
    let exponent = BigUint::from(0x10001u32);
    let mut key = Vec::with_capacity(78);
    for chunk in source.chunks(40) {
        let mut out = BigUint::from_bytes_le(chunk)
            .modpow(&exponent, &modulus)
            .to_bytes_le();
        out.resize(39, 0);
        key.extend_from_slice(&out);
    }
    key.truncate(56);
    key
}

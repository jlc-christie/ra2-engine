//! CSF string tables: the game's localised text, keyed by label.

use std::collections::HashMap;

pub struct Csf {
    strings: HashMap<String, String>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let out = self.data.get(self.at..self.at + len)?;
        self.at += len;
        Some(out)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
}

impl Csf {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(4)? != b" FSC" {
            return None;
        }
        let _version = r.u32()?;
        let label_count = r.u32()?;
        r.bytes(12)?;
        let mut strings = HashMap::new();
        for _ in 0..label_count {
            if r.bytes(4)? != b" LBL" {
                return None;
            }
            let value_count = r.u32()?;
            let name_len = r.u32()? as usize;
            let name = String::from_utf8_lossy(r.bytes(name_len)?).to_uppercase();
            for index in 0..value_count {
                let tag = r.bytes(4)?;
                let len = r.u32()? as usize;
                // Text is UTF-16 with every byte inverted.
                let units: Vec<u16> = r
                    .bytes(len * 2)?
                    .as_chunks::<2>().0.iter()
                    .map(|c| !u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                if tag == b"WRTS" {
                    let extra = r.u32()? as usize;
                    r.bytes(extra)?;
                }
                if index == 0 {
                    strings.insert(name.clone(), String::from_utf16_lossy(&units));
                }
            }
        }
        Some(Self { strings })
    }

    /// Looks up a label, case-insensitively.
    pub fn get(&self, label: &str) -> Option<&str> {
        self.strings.get(&label.to_uppercase()).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.strings.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

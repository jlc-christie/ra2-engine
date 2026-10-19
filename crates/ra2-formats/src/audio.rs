//! Sound: the `audio.idx`/`audio.bag` sound bank, RIFF wave files, and the IMA
//! ADPCM compression both use.
//!
//! DATA: the layouts here are read off the game's files. `audio.idx` starts with
//! "GABA", a version (2) and an entry count; each 36-byte entry is a 16-byte name
//! followed by offset, size, sample rate, flags and block size. Flag bit 0 is
//! stereo, bit 1 plain 16-bit samples, bit 3 IMA ADPCM.

/// Decoded sound: interleaved signed 16-bit samples.
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

const FLAG_STEREO: u32 = 1;
const FLAG_PCM: u32 = 2;
const FLAG_ADPCM: u32 = 8;

struct Entry {
    name: String,
    offset: usize,
    size: usize,
    rate: u32,
    flags: u32,
    block: usize,
}

/// The index of a sound bank.
pub struct AudioIndex {
    entries: Vec<Entry>,
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

impl AudioIndex {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.get(..4)? != b"GABA" {
            return None;
        }
        let count = u32_at(data, 8)? as usize;
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let at = 12 + index * 36;
            let name = data.get(at..at + 16)?;
            let name = name.split(|&b| b == 0).next().unwrap_or(&[]);
            entries.push(Entry {
                name: String::from_utf8_lossy(name).to_lowercase(),
                offset: u32_at(data, at + 16)? as usize,
                size: u32_at(data, at + 20)? as usize,
                rate: u32_at(data, at + 24)?,
                flags: u32_at(data, at + 28)?,
                block: u32_at(data, at + 32)? as usize,
            });
        }
        Some(Self { entries })
    }

    /// Decodes a named sound out of the bank's data.
    pub fn decode(&self, name: &str, bag: &[u8]) -> Option<Pcm> {
        let entry = self.entries.iter().find(|entry| entry.name.eq_ignore_ascii_case(name))?;
        let data = bag.get(entry.offset..entry.offset + entry.size)?;
        let channels = if entry.flags & FLAG_STEREO != 0 { 2 } else { 1 };
        let samples = if entry.flags & FLAG_ADPCM != 0 {
            decode_ima_adpcm(data, channels, entry.block)
        } else if entry.flags & FLAG_PCM != 0 {
            data.as_chunks::<2>().0.iter().map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect()
        } else {
            return None;
        };
        Some(Pcm {
            rate: entry.rate,
            channels: channels as u16,
            samples,
        })
    }
}

const INDEX_STEP: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
const STEP_SIZE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73,
    80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494,
    544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499,
    2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442,
    11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

struct Channel {
    predictor: i32,
    index: i32,
}

impl Channel {
    fn step(&mut self, nibble: u8) -> i16 {
        let step = STEP_SIZE[self.index as usize];
        let mut delta = step >> 3;
        if nibble & 1 != 0 {
            delta += step >> 2;
        }
        if nibble & 2 != 0 {
            delta += step >> 1;
        }
        if nibble & 4 != 0 {
            delta += step;
        }
        if nibble & 8 != 0 {
            delta = -delta;
        }
        self.predictor = (self.predictor + delta).clamp(-32768, 32767);
        self.index = (self.index + INDEX_STEP[nibble as usize]).clamp(0, 88);
        self.predictor as i16
    }
}

/// Decodes the Microsoft flavour of IMA ADPCM: blocks that each start with a
/// four-byte header per channel (first sample and step index), then four-bit
/// codes, low half of each byte first; with two channels, in alternating groups of
/// four bytes.
pub fn decode_ima_adpcm(data: &[u8], channels: usize, block: usize) -> Vec<i16> {
    let mut out = Vec::with_capacity(data.len() * 4);
    if channels == 0 || block <= 4 * channels {
        return out;
    }
    for block in data.chunks(block) {
        if block.len() < 4 * channels {
            break;
        }
        let mut state: Vec<Channel> = (0..channels)
            .map(|c| Channel {
                predictor: i16::from_le_bytes([block[c * 4], block[c * 4 + 1]]) as i32,
                index: (block[c * 4 + 2] as i32).clamp(0, 88),
            })
            .collect();
        for channel in &state {
            out.push(channel.predictor as i16);
        }
        let body = &block[4 * channels..];
        if channels == 1 {
            for &byte in body {
                out.push(state[0].step(byte & 15));
                out.push(state[0].step(byte >> 4));
            }
        } else {
            for group in body.chunks_exact(4 * channels) {
                let start = out.len();
                out.resize(start + 8 * channels, 0);
                for (c, channel) in state.iter_mut().enumerate() {
                    for (i, &byte) in group[c * 4..c * 4 + 4].iter().enumerate() {
                        out[start + (i * 2) * channels + c] = channel.step(byte & 15);
                        out[start + (i * 2 + 1) * channels + c] = channel.step(byte >> 4);
                    }
                }
            }
        }
    }
    out
}

impl Pcm {
    /// Reads a RIFF wave file holding plain 16-bit samples or IMA ADPCM.
    pub fn from_wav(data: &[u8]) -> Option<Self> {
        if data.get(..4)? != b"RIFF" || data.get(8..12)? != b"WAVE" {
            return None;
        }
        let mut at = 12;
        let mut format = None;
        while at + 8 <= data.len() {
            let id = &data[at..at + 4];
            let size = u32_at(data, at + 4)? as usize;
            let body = data.get(at + 8..(at + 8 + size).min(data.len()))?;
            if id == b"fmt " {
                let tag = u16::from_le_bytes([*body.first()?, *body.get(1)?]);
                let channels = u16::from_le_bytes([*body.get(2)?, *body.get(3)?]);
                let rate = u32_at(body, 4)?;
                let block = u16::from_le_bytes([*body.get(12)?, *body.get(13)?]) as usize;
                format = Some((tag, channels, rate, block));
            } else if id == b"data" {
                let (tag, channels, rate, block) = format?;
                let samples = match tag {
                    1 => body.as_chunks::<2>().0.iter().map(|p| i16::from_le_bytes([p[0], p[1]])).collect(),
                    0x11 => decode_ima_adpcm(body, channels as usize, block),
                    _ => return None,
                };
                return Some(Self { rate, channels, samples });
            }
            at += 8 + size + (size & 1);
        }
        None
    }

    /// Writes the sound as a RIFF wave file of plain 16-bit samples.
    pub fn to_wav(&self) -> Vec<u8> {
        let bytes = self.samples.len() * 2;
        let mut out = Vec::with_capacity(44 + bytes);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + bytes) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&(self.rate * self.channels as u32 * 2).to_le_bytes());
        out.extend_from_slice(&(self.channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(bytes as u32).to_le_bytes());
        for sample in &self.samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adpcm_block_starts_with_its_header_sample_and_follows_the_codes() {
        // Header: first sample 100, step index 0; then codes 4 (up a step) and
        // 12 (down a larger step, the first code having raised the step size).
        let block = [100, 0, 0, 0, 0xc4];
        let samples = decode_ima_adpcm(&block, 1, 5);
        assert_eq!(samples, vec![100, 107, 97]);
    }

    #[test]
    fn a_wave_file_round_trips() {
        let pcm = Pcm { rate: 22050, channels: 2, samples: vec![1, -2, 300, -400] };
        let back = Pcm::from_wav(&pcm.to_wav()).unwrap();
        assert_eq!((back.rate, back.channels, back.samples), (22050, 2, vec![1, -2, 300, -400]));
    }
}

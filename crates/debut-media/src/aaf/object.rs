//! AAF objects as stored: a class id and properties, each raw bytes or a
//! reference, serialized the way the AAF SDK and pyaaf2 lay them out in a
//! compound file (one storage per object, a `properties` stream, index
//! streams for collections, a root table of weak-reference target paths).

use super::cfb::Entry;
use serde_json::Value;

/// A 16-byte AUID or a 32-byte MobID, in AAF's stored (little-endian) form.
pub type Key = Vec<u8>;

#[derive(Clone, Debug)]
pub struct Obj {
    pub class: [u8; 16],
    pub props: Vec<(u16, Prop)>,
}

#[derive(Clone, Debug)]
pub enum Prop {
    Data(Vec<u8>),
    /// A contained object, stored in a sub-storage of this name.
    Strong(String, Box<Obj>),
    /// An ordered list of contained objects (`name{i}` storages + `name index`).
    Vector(String, Vec<Obj>),
    /// Contained objects keyed by a unique property (`key_pid`).
    Set {
        name: String,
        key_pid: u16,
        key_size: u8,
        items: Vec<(Key, Obj)>,
    },
    /// A reference by key to an object in the collection at `path` (pids from the root).
    Weak {
        path: Vec<u16>,
        key_pid: u16,
        key: Key,
    },
    /// Several weak references (a vector, or a set when `set`).
    WeakArray {
        set: bool,
        name: String,
        path: Vec<u16>,
        key_pid: u16,
        key_size: u8,
        keys: Vec<Key>,
    },
}

const SF_DATA: u16 = 0x82;
const SF_STRONG: u16 = 0x22;
const SF_STRONG_VECTOR: u16 = 0x32;
const SF_STRONG_SET: u16 = 0x3A;
const SF_WEAK: u16 = 0x02;
const SF_WEAK_VECTOR: u16 = 0x12;
const SF_WEAK_SET: u16 = 0x1A;
const PROPERTY_VERSION: u8 = 32;

/// AUID text ("0d010101-0101-2f00-060e-2b3402060101") to stored bytes.
pub fn auid(text: &str) -> [u8; 16] {
    let hex: Vec<u8> = text
        .bytes()
        .filter(|b| *b != b'-')
        .collect::<Vec<_>>()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect();
    assert_eq!(hex.len(), 16, "bad AUID {text}");
    let mut out = [0u8; 16];
    // data1 (u32), data2 and data3 (u16) little-endian; data4 as is.
    out[..4].copy_from_slice(&[hex[3], hex[2], hex[1], hex[0]]);
    out[4..6].copy_from_slice(&[hex[5], hex[4]]);
    out[6..8].copy_from_slice(&[hex[7], hex[6]]);
    out[8..].copy_from_slice(&hex[8..]);
    out
}

/// A storage or index name: `name-pid` squeezed to fit `size` (pyaaf2's and
/// the SDK's convention).
pub fn mangle(name: &str, pid: u16, size: usize) -> String {
    let p = format!("{pid:x}");
    let max = size - p.len() - 2;
    let chars: Vec<char> = name.chars().collect();
    let squeezed: String = if chars.len() <= max {
        name.to_string()
    } else {
        let half = max / 2;
        (0..max)
            .map(|i| match i.cmp(&half) {
                std::cmp::Ordering::Less => chars[i],
                std::cmp::Ordering::Equal => '-',
                std::cmp::Ordering::Greater => chars[chars.len() - (max - i)],
            })
            .collect()
    };
    format!("{squeezed}-{p}")
}

pub fn utf16z(s: &str) -> Vec<u8> {
    let mut out: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    out.extend_from_slice(&[0, 0]);
    out
}

fn hex(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd hex string".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// An object dumped by `tools/aaf_baseline.py`.
pub fn from_json(v: &Value) -> Result<Obj, String> {
    let bad = || "malformed AAF baseline".to_string();
    let class: [u8; 16] = hex(v["c"].as_str().ok_or_else(bad)?)?
        .try_into()
        .map_err(|_| bad())?;
    let mut props = Vec::new();
    for p in v["p"].as_array().ok_or_else(bad)? {
        let a = p.as_array().ok_or_else(bad)?;
        let pid = a[0].as_u64().ok_or_else(bad)? as u16;
        let s = |i: usize| a[i].as_str().ok_or_else(bad);
        let n = |i: usize| a[i].as_u64().ok_or_else(bad);
        let path = |i: usize| -> Result<Vec<u16>, String> {
            a[i].as_array()
                .ok_or_else(bad)?
                .iter()
                .map(|x| x.as_u64().map(|v| v as u16).ok_or_else(bad))
                .collect()
        };
        let prop = match s(1)? {
            "d" => Prop::Data(hex(s(2)?)?),
            "s" => Prop::Strong(s(2)?.to_string(), Box::new(from_json(&a[3])?)),
            "v" => Prop::Vector(
                s(2)?.to_string(),
                a[3].as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .map(from_json)
                    .collect::<Result<_, _>>()?,
            ),
            "S" => Prop::Set {
                name: s(2)?.to_string(),
                key_pid: n(3)? as u16,
                key_size: n(4)? as u8,
                items: a[5]
                    .as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .map(|it| -> Result<(Key, Obj), String> {
                        Ok((hex(it[0].as_str().ok_or_else(bad)?)?, from_json(&it[1])?))
                    })
                    .collect::<Result<_, _>>()?,
            },
            "w" => Prop::Weak {
                path: path(2)?,
                key_pid: n(3)? as u16,
                key: hex(s(4)?)?,
            },
            kind @ ("W" | "WS") => Prop::WeakArray {
                set: kind == "WS",
                name: s(2)?.to_string(),
                path: path(3)?,
                key_pid: n(4)? as u16,
                key_size: n(5)? as u8,
                keys: a[6]
                    .as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .map(|k| hex(k.as_str().ok_or_else(bad)?))
                    .collect::<Result<_, _>>()?,
            },
            _ => return Err(bad()),
        };
        props.push((pid, prop));
    }
    Ok(Obj { class, props })
}

/// Weak-reference target paths, in first-use order (the `referenced
/// properties` stream; weak references store an index into it).
#[derive(Default)]
pub struct WeakTable(Vec<Vec<u16>>);

impl WeakTable {
    fn index(&mut self, path: &[u16]) -> u16 {
        match self.0.iter().position(|p| p == path) {
            Some(i) => i as u16,
            None => {
                self.0.push(path.to_vec());
                (self.0.len() - 1) as u16
            }
        }
    }

    pub fn stream(&self) -> Vec<u8> {
        let mut out = vec![0x4c];
        out.extend_from_slice(&(self.0.len() as u16).to_le_bytes());
        let pids: usize = self.0.iter().map(|p| p.len() + 1).sum();
        out.extend_from_slice(&(pids as u32).to_le_bytes());
        for path in &self.0 {
            for pid in path {
                out.extend_from_slice(&pid.to_le_bytes());
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }
}

fn index_header(count: usize) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&(count as u32).to_le_bytes());
    f.extend_from_slice(&(count as u32).to_le_bytes()); // next free key
    f.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // last free key
    f
}

/// `obj`'s children in its storage: its `properties` stream, contained
/// objects and index streams.
pub fn entries(obj: &Obj, weak: &mut WeakTable) -> Vec<Entry> {
    let mut children = Vec::new();
    let mut table = Vec::new();
    let mut data = Vec::new();
    for (pid, prop) in &obj.props {
        let (form, bytes) = match prop {
            Prop::Data(d) => (SF_DATA, d.clone()),
            Prop::Strong(name, child) => {
                children.push(storage(name.clone(), child, weak));
                (SF_STRONG, utf16z(name))
            }
            Prop::Vector(name, items) => {
                let mut index = index_header(items.len());
                for (i, item) in items.iter().enumerate() {
                    index.extend_from_slice(&(i as u32).to_le_bytes());
                    children.push(storage(format!("{name}{{{i:x}}}"), item, weak));
                }
                children.push(Entry::Stream {
                    name: format!("{name} index"),
                    data: index,
                });
                (SF_STRONG_VECTOR, utf16z(name))
            }
            Prop::Set {
                name,
                key_pid,
                key_size,
                items,
            } => {
                let mut index = index_header(items.len());
                index.extend_from_slice(&key_pid.to_le_bytes());
                index.push(*key_size);
                for (i, (key, item)) in items.iter().enumerate() {
                    index.extend_from_slice(&(i as u32).to_le_bytes());
                    index.extend_from_slice(&1u32.to_le_bytes()); // reference count
                    index.extend_from_slice(key);
                    children.push(storage(format!("{name}{{{i:x}}}"), item, weak));
                }
                children.push(Entry::Stream {
                    name: format!("{name} index"),
                    data: index,
                });
                (SF_STRONG_SET, utf16z(name))
            }
            Prop::Weak { path, key_pid, key } => {
                let mut b = Vec::new();
                b.extend_from_slice(&weak.index(path).to_le_bytes());
                b.extend_from_slice(&key_pid.to_le_bytes());
                b.push(key.len() as u8);
                b.extend_from_slice(key);
                (SF_WEAK, b)
            }
            Prop::WeakArray {
                set,
                name,
                path,
                key_pid,
                key_size,
                keys,
            } => {
                let mut index = Vec::new();
                index.extend_from_slice(&(keys.len() as u32).to_le_bytes());
                index.extend_from_slice(&weak.index(path).to_le_bytes());
                index.extend_from_slice(&key_pid.to_le_bytes());
                index.push(*key_size);
                for k in keys {
                    index.extend_from_slice(k);
                }
                children.push(Entry::Stream {
                    name: format!("{name} index"),
                    data: index,
                });
                (
                    if *set { SF_WEAK_SET } else { SF_WEAK_VECTOR },
                    utf16z(name),
                )
            }
        };
        table.push((*pid, form, bytes.len() as u16));
        data.extend_from_slice(&bytes);
    }
    let mut stream = vec![0x4c, PROPERTY_VERSION];
    stream.extend_from_slice(&(table.len() as u16).to_le_bytes());
    for (pid, form, len) in table {
        stream.extend_from_slice(&pid.to_le_bytes());
        stream.extend_from_slice(&form.to_le_bytes());
        stream.extend_from_slice(&len.to_le_bytes());
    }
    stream.extend_from_slice(&data);
    children.push(Entry::Stream {
        name: "properties".into(),
        data: stream,
    });
    children
}

fn storage(name: String, obj: &Obj, weak: &mut WeakTable) -> Entry {
    Entry::Storage {
        name,
        clsid: obj.class,
        children: entries(obj, weak),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ids_match_pyaaf2() {
        assert_eq!(mangle("Header", 2, 32), "Header-2");
        assert_eq!(
            mangle("IdentificationList", 0x3b06, 22),
            "Identifi-ionList-3b06"
        );
        assert_eq!(
            auid("0d010101-0101-2f00-060e-2b3402060101"),
            [
                0x01, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x00, 0x2f, 0x06, 0x0e, 0x2b, 0x34, 0x02, 0x06,
                0x01, 0x01
            ]
        );
    }
}

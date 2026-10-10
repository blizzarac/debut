//! A writer for Microsoft compound files (MS-CFB, version 3), the container
//! AAF files use: a small file system of storages (directories) and streams.
//! Streams under 4096 bytes live in the mini stream, as the format requires.

/// A storage (with children) or a stream (with bytes).
pub enum Entry {
    Storage {
        name: String,
        clsid: [u8; 16],
        children: Vec<Entry>,
    },
    Stream {
        name: String,
        data: Vec<u8>,
    },
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::Storage { name, .. } | Entry::Stream { name, .. } => name,
        }
    }
}

const SECTOR: usize = 512;
const MINI_SECTOR: usize = 64;
const MINI_CUTOFF: usize = 4096;
const FREE: u32 = 0xFFFF_FFFF;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const DIFAT_SECT: u32 = 0xFFFF_FFFC;
const NO_STREAM: u32 = 0xFFFF_FFFF;
const IDS_PER_SECTOR: usize = SECTOR / 4;

struct Dir {
    name: Vec<u16>,
    kind: u8, // 1 storage, 2 stream, 5 root
    clsid: [u8; 16],
    left: u32,
    right: u32,
    child: u32,
    black: bool,
    start: u32,
    size: u64,
}

/// CFB's sibling order: shorter names first, then by upper-cased code units.
fn cfb_cmp(a: &[u16], b: &[u16]) -> std::cmp::Ordering {
    let up = |c: u16| {
        if (b'a' as u16..=b'z' as u16).contains(&c) {
            c - 32
        } else {
            c
        }
    };
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().map(|&c| up(c)).cmp(b.iter().map(|&c| up(c))))
}

/// The bytes of a compound file holding `children` under the root storage
/// (whose class id is `root_clsid`). Names longer than 31 UTF-16 units are an error.
pub fn write(root_clsid: [u8; 16], children: Vec<Entry>) -> Result<Vec<u8>, String> {
    let mut dirs: Vec<Dir> = Vec::new();
    let mut streams: Vec<(usize, Vec<u8>)> = Vec::new(); // (dir id, data)
    dirs.push(Dir {
        name: "Root Entry".encode_utf16().collect(),
        kind: 5,
        clsid: root_clsid,
        left: NO_STREAM,
        right: NO_STREAM,
        child: NO_STREAM,
        black: true,
        start: END_OF_CHAIN,
        size: 0,
    });
    add_children(&mut dirs, &mut streams, 0, children)?;

    // Small streams into the mini stream, the rest into sectors.
    let mut mini = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut big: Vec<(usize, Vec<u8>)> = Vec::new();
    for (id, data) in streams {
        dirs[id].size = data.len() as u64;
        if data.is_empty() {
            dirs[id].start = END_OF_CHAIN;
        } else if data.len() < MINI_CUTOFF {
            let first = mini_fat.len();
            let n = data.len().div_ceil(MINI_SECTOR);
            for i in 0..n {
                mini_fat.push(if i + 1 == n {
                    END_OF_CHAIN
                } else {
                    (first + i + 1) as u32
                });
            }
            dirs[id].start = first as u32;
            mini.extend_from_slice(&data);
            mini.resize(mini_fat.len() * MINI_SECTOR, 0);
        } else {
            big.push((id, data));
        }
    }

    // Sector layout: big streams, mini stream, mini FAT, directory, FAT, DIFAT.
    let mut sectors: Vec<u8> = Vec::new();
    let mut fat: Vec<u32> = Vec::new();
    let chain = |sectors: &mut Vec<u8>, fat: &mut Vec<u32>, data: &[u8]| -> u32 {
        if data.is_empty() {
            return END_OF_CHAIN;
        }
        let first = fat.len();
        let n = data.len().div_ceil(SECTOR);
        for i in 0..n {
            fat.push(if i + 1 == n {
                END_OF_CHAIN
            } else {
                (first + i + 1) as u32
            });
        }
        sectors.extend_from_slice(data);
        sectors.resize(fat.len() * SECTOR, 0);
        first as u32
    };
    for (id, data) in &big {
        dirs[*id].start = chain(&mut sectors, &mut fat, data);
    }
    dirs[0].start = chain(&mut sectors, &mut fat, &mini);
    dirs[0].size = mini.len() as u64;
    let mini_fat_bytes: Vec<u8> = mini_fat.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut mini_fat_bytes = mini_fat_bytes;
    if !mini_fat_bytes.is_empty() {
        mini_fat_bytes.resize(mini_fat_bytes.len().div_ceil(SECTOR) * SECTOR, 0xFF);
    }
    let mini_fat_start = chain(&mut sectors, &mut fat, &mini_fat_bytes);
    let mini_fat_sectors = mini_fat_bytes.len() / SECTOR;
    let dir_bytes = encode_dirs(&dirs);
    let dir_start = chain(&mut sectors, &mut fat, &dir_bytes);

    // FAT and DIFAT sectors must also be counted in the FAT.
    let content = fat.len();
    let (mut n_fat, mut n_difat) = (1usize, 0usize);
    loop {
        let total = content + n_fat + n_difat;
        let need_fat = total.div_ceil(IDS_PER_SECTOR);
        let need_difat = need_fat.saturating_sub(109).div_ceil(IDS_PER_SECTOR - 1);
        if need_fat == n_fat && need_difat == n_difat {
            break;
        }
        n_fat = need_fat;
        n_difat = need_difat;
    }
    let fat_first = content;
    fat.extend(std::iter::repeat_n(FAT_SECT, n_fat));
    let difat_first = fat.len();
    fat.extend(std::iter::repeat_n(DIFAT_SECT, n_difat));
    fat.resize(n_fat * IDS_PER_SECTOR, FREE);
    for v in &fat {
        sectors.extend_from_slice(&v.to_le_bytes());
    }
    // DIFAT sectors: FAT sector ids past the first 109, chained by their last slot.
    let fat_ids: Vec<u32> = (0..n_fat).map(|i| (fat_first + i) as u32).collect();
    let overflow = fat_ids.get(109..).unwrap_or(&[]);
    for d in 0..n_difat {
        let mut slots = vec![FREE; IDS_PER_SECTOR];
        let chunk = overflow
            .iter()
            .skip(d * (IDS_PER_SECTOR - 1))
            .take(IDS_PER_SECTOR - 1);
        for (i, id) in chunk.enumerate() {
            slots[i] = *id;
        }
        slots[IDS_PER_SECTOR - 1] = if d + 1 == n_difat {
            END_OF_CHAIN
        } else {
            (difat_first + d + 1) as u32
        };
        for v in slots {
            sectors.extend_from_slice(&v.to_le_bytes());
        }
    }

    let mut header = Vec::with_capacity(SECTOR);
    header.extend_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    header.extend_from_slice(&[0; 16]); // clsid
    header.extend_from_slice(&0x003Eu16.to_le_bytes()); // minor version
    header.extend_from_slice(&3u16.to_le_bytes()); // major version
    header.extend_from_slice(&0xFFFEu16.to_le_bytes()); // byte order
    header.extend_from_slice(&9u16.to_le_bytes()); // sector shift
    header.extend_from_slice(&6u16.to_le_bytes()); // mini sector shift
    header.extend_from_slice(&[0; 6]);
    header.extend_from_slice(&0u32.to_le_bytes()); // directory sectors (v3: 0)
    header.extend_from_slice(&(n_fat as u32).to_le_bytes());
    header.extend_from_slice(&dir_start.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); // transaction signature
    header.extend_from_slice(&(MINI_CUTOFF as u32).to_le_bytes());
    header.extend_from_slice(&mini_fat_start.to_le_bytes());
    header.extend_from_slice(&(mini_fat_sectors as u32).to_le_bytes());
    let difat_start = if n_difat > 0 {
        difat_first as u32
    } else {
        END_OF_CHAIN
    };
    header.extend_from_slice(&difat_start.to_le_bytes());
    header.extend_from_slice(&(n_difat as u32).to_le_bytes());
    for i in 0..109 {
        let v = fat_ids.get(i).copied().unwrap_or(FREE);
        header.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(header.len(), SECTOR);
    header.extend_from_slice(&sectors);
    Ok(header)
}

/// Add `children` under directory `parent`, as a balanced red-black tree.
fn add_children(
    dirs: &mut Vec<Dir>,
    streams: &mut Vec<(usize, Vec<u8>)>,
    parent: usize,
    children: Vec<Entry>,
) -> Result<(), String> {
    let mut ids = Vec::with_capacity(children.len());
    let mut nested = Vec::new();
    for entry in children {
        let name: Vec<u16> = entry.name().encode_utf16().collect();
        if name.len() > 31 {
            return Err(format!("compound file name too long: {}", entry.name()));
        }
        let id = dirs.len();
        match entry {
            Entry::Storage {
                clsid, children, ..
            } => {
                dirs.push(Dir {
                    name,
                    kind: 1,
                    clsid,
                    left: NO_STREAM,
                    right: NO_STREAM,
                    child: NO_STREAM,
                    black: true,
                    start: 0,
                    size: 0,
                });
                nested.push((id, children));
            }
            Entry::Stream { data, .. } => {
                dirs.push(Dir {
                    name,
                    kind: 2,
                    clsid: [0; 16],
                    left: NO_STREAM,
                    right: NO_STREAM,
                    child: NO_STREAM,
                    black: true,
                    start: END_OF_CHAIN,
                    size: 0,
                });
                streams.push((id, data));
            }
        }
        ids.push(id);
    }
    ids.sort_by(|&a, &b| cfb_cmp(&dirs[a].name, &dirs[b].name));
    for w in ids.windows(2) {
        if cfb_cmp(&dirs[w[0]].name, &dirs[w[1]].name).is_eq() {
            return Err("duplicate name in a compound file storage".into());
        }
    }
    // A middle-split tree has all its leaves on its last two levels; when it
    // is not perfect, the last level is red, so every path to a leaf crosses
    // the same number of black nodes.
    let n = ids.len();
    let depth = (usize::BITS - n.leading_zeros()) as usize; // levels needed
    let red = if (n + 1).is_power_of_two() {
        usize::MAX
    } else {
        depth
    };
    dirs[parent].child = build(dirs, &ids, 1, red);
    for (id, children) in nested {
        add_children(dirs, streams, id, children)?;
    }
    Ok(())
}

fn build(dirs: &mut [Dir], ids: &[usize], level: usize, red: usize) -> u32 {
    if ids.is_empty() {
        return NO_STREAM;
    }
    let mid = ids.len() / 2;
    let id = ids[mid];
    let left = build(dirs, &ids[..mid], level + 1, red);
    let right = build(dirs, &ids[mid + 1..], level + 1, red);
    let d = &mut dirs[id];
    d.left = left;
    d.right = right;
    d.black = level != red;
    id as u32
}

fn encode_dirs(dirs: &[Dir]) -> Vec<u8> {
    let mut out = Vec::with_capacity(dirs.len().div_ceil(4) * SECTOR);
    for d in dirs {
        let mut name = [0u8; 64];
        for (i, c) in d.name.iter().enumerate() {
            name[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&name);
        out.extend_from_slice(&(((d.name.len() + 1) * 2) as u16).to_le_bytes());
        out.push(d.kind);
        out.push(if d.black { 1 } else { 0 });
        out.extend_from_slice(&d.left.to_le_bytes());
        out.extend_from_slice(&d.right.to_le_bytes());
        out.extend_from_slice(&d.child.to_le_bytes());
        out.extend_from_slice(&d.clsid);
        out.extend_from_slice(&0u32.to_le_bytes()); // state bits
        out.extend_from_slice(&0u64.to_le_bytes()); // created
        out.extend_from_slice(&0u64.to_le_bytes()); // modified
        out.extend_from_slice(&d.start.to_le_bytes());
        out.extend_from_slice(&d.size.to_le_bytes());
    }
    // Pad to whole sectors with empty (unused) entries.
    while out.len() % SECTOR != 0 {
        let mut empty = [0u8; 128];
        empty[68..80].copy_from_slice(&[0xFF; 12]); // left/right/child = NOSTREAM
        out.extend_from_slice(&empty);
    }
    out
}

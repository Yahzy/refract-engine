//! Addon archives (`.gma`), as downloaded from the Steam Workshop.
//!
//! Layout: `GMAD`, version u8, steamid u64, timestamp u64, [required content
//! strings (version > 1)], name, description, author, addon version i32,
//! then a file table `(index u32 != 0, name, size i64, crc u32)*, 0u32`
//! followed by every file's data, in table order.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub struct Gma {
    pub path: PathBuf,
    pub name: String,
    /// (name as stored, absolute offset in the file, size).
    pub files: Vec<(String, u64, u64)>,
}

impl Gma {
    /// Reads the header and file table only; data is read on demand.
    pub fn open(path: &Path) -> Result<Gma> {
        let mut reader = BufReader::new(File::open(path).with_context(|| format!("opening {}", path.display()))?);
        let mut ident = [0u8; 4];
        reader.read_exact(&mut ident)?;
        if &ident != b"GMAD" {
            bail!("{} is not a GMA archive", path.display());
        }
        let version = read_u8(&mut reader)?;
        let _steamid = read_u64(&mut reader)?;
        let _timestamp = read_u64(&mut reader)?;
        if version > 1 {
            while !read_cstring(&mut reader)?.is_empty() {}
        }
        let name = read_cstring(&mut reader)?;
        let _description = read_cstring(&mut reader)?;
        let _author = read_cstring(&mut reader)?;
        let _addon_version = read_u32(&mut reader)?;

        let mut entries = Vec::new();
        while read_u32(&mut reader)? != 0 {
            let file_name = read_cstring(&mut reader)?;
            let size = read_u64(&mut reader)?;
            let _crc = read_u32(&mut reader)?;
            entries.push((file_name, size));
        }
        let mut offset = reader.stream_position()?;
        let files = entries
            .into_iter()
            .map(|(file_name, size)| {
                let entry = (file_name, offset, size);
                offset += size;
                entry
            })
            .collect();
        Ok(Gma { path: path.to_owned(), name, files })
    }

    pub fn read(&self, offset: u64, size: u64) -> std::io::Result<Vec<u8>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0u8; size as usize];
        file.read_exact(&mut data)?;
        Ok(data)
    }
}

fn read_u8(r: &mut impl Read) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_cstring(r: &mut impl BufRead) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    r.read_until(0, &mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_file_table() {
        let mut data = b"GMAD".to_vec();
        data.push(3);
        data.extend_from_slice(&[0; 16]); // steamid + timestamp
        data.push(0); // no required content
        data.extend_from_slice(b"Test Addon\0desc\0author\0");
        data.extend_from_slice(&1u32.to_le_bytes());
        for (i, (name, content)) in [("materials/a.vmt", "abc"), ("lua/b.lua", "print(1)")].iter().enumerate() {
            data.extend_from_slice(&(i as u32 + 1).to_le_bytes());
            data.extend_from_slice(name.as_bytes());
            data.push(0);
            data.extend_from_slice(&(content.len() as u64).to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
        }
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"abcprint(1)");

        let path = std::env::temp_dir().join(format!("refract-gma-test-{}.gma", std::process::id()));
        std::fs::write(&path, &data).unwrap();
        let gma = Gma::open(&path).unwrap();
        assert_eq!(gma.name, "Test Addon");
        assert_eq!(gma.files.len(), 2);
        let (_, offset, size) = &gma.files[1];
        assert_eq!(gma.read(*offset, *size).unwrap(), b"print(1)");
        std::fs::remove_file(&path).unwrap();
    }
}

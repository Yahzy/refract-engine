//! The binary container of maps and models.
//!
//! ```text
//! header   magic "RFCT", kind (4 bytes), version (u32), section count (u32)
//! table    per section: id (4 bytes), reserved (u32), offset (u64), size (u64)
//! data     each section starts at a multiple of 16 bytes
//! ```
//!
//! Everything is little-endian. Sections hold arrays of plain structs, cast
//! in place, or UTF-8 text.

use bytemuck::{Pod, Zeroable};

use crate::AssetError;

const MAGIC: [u8; 4] = *b"RFCT";
const ALIGN: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Header {
    magic: [u8; 4],
    kind: [u8; 4],
    version: u32,
    section_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SectionEntry {
    id: [u8; 4],
    reserved: u32,
    offset: u64,
    size: u64,
}

/// A container read in place from its bytes.
pub struct Container<'a> {
    bytes: &'a [u8],
    sections: &'a [SectionEntry],
}

impl<'a> Container<'a> {
    pub fn parse(bytes: &'a [u8], kind: [u8; 4], version: u32, kind_name: &'static str) -> Result<Self, AssetError> {
        let header: &Header = bytes.get(..size_of::<Header>()).and_then(|b| bytemuck::try_from_bytes(b).ok()).ok_or(AssetError::Corrupt("header"))?;
        if header.magic != MAGIC || header.kind != kind {
            return Err(AssetError::WrongKind { expected: kind_name });
        }
        if header.version != version {
            return Err(AssetError::Version { found: header.version, expected: version });
        }
        let table_end = size_of::<Header>() + header.section_count as usize * size_of::<SectionEntry>();
        let sections = bytes.get(size_of::<Header>()..table_end).and_then(|b| bytemuck::try_cast_slice(b).ok()).ok_or(AssetError::Corrupt("section table"))?;
        Ok(Self { bytes, sections })
    }

    pub fn raw(&self, id: [u8; 4]) -> Option<&'a [u8]> {
        let entry = self.sections.iter().find(|s| s.id == id)?;
        self.bytes.get(entry.offset as usize..(entry.offset + entry.size) as usize)
    }

    /// A section as an array of `T`.
    pub fn array<T: Pod>(&self, id: [u8; 4], name: &'static str) -> Result<&'a [T], AssetError> {
        let raw = self.raw(id).ok_or(AssetError::MissingSection(name))?;
        bytemuck::try_cast_slice(raw).map_err(|_| AssetError::Corrupt(name))
    }

    /// A section holding one `T`.
    pub fn single<T: Pod>(&self, id: [u8; 4], name: &'static str) -> Result<&'a T, AssetError> {
        self.array(id, name)?.first().ok_or(AssetError::Corrupt(name))
    }

    pub fn text(&self, id: [u8; 4], name: &'static str) -> Result<&'a str, AssetError> {
        std::str::from_utf8(self.raw(id).ok_or(AssetError::MissingSection(name))?).map_err(|_| AssetError::Corrupt(name))
    }
}

/// Builds a container section by section.
pub struct ContainerWriter {
    kind: [u8; 4],
    version: u32,
    sections: Vec<([u8; 4], Vec<u8>)>,
}

impl ContainerWriter {
    pub fn new(kind: [u8; 4], version: u32) -> Self {
        Self { kind, version, sections: Vec::new() }
    }

    pub fn array<T: Pod>(&mut self, id: [u8; 4], items: &[T]) -> &mut Self {
        self.sections.push((id, bytemuck::cast_slice(items).to_vec()));
        self
    }

    pub fn text(&mut self, id: [u8; 4], text: &str) -> &mut Self {
        self.sections.push((id, text.as_bytes().to_vec()));
        self
    }

    pub fn finish(&self) -> Vec<u8> {
        let header = Header { magic: MAGIC, kind: self.kind, version: self.version, section_count: self.sections.len() as u32 };
        let mut offset = (size_of::<Header>() + self.sections.len() * size_of::<SectionEntry>()).next_multiple_of(ALIGN);
        let mut table = Vec::new();
        for (id, data) in &self.sections {
            table.push(SectionEntry { id: *id, reserved: 0, offset: offset as u64, size: data.len() as u64 });
            offset = (offset + data.len()).next_multiple_of(ALIGN);
        }
        let mut bytes = Vec::with_capacity(offset);
        bytes.extend_from_slice(bytemuck::bytes_of(&header));
        bytes.extend_from_slice(bytemuck::cast_slice(&table));
        for ((_, data), entry) in self.sections.iter().zip(&table) {
            bytes.resize(entry.offset as usize, 0);
            bytes.extend_from_slice(data);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut writer = ContainerWriter::new(*b"TEST", 3);
        writer.array(*b"NUMS", &[1u32, 2, 3]).text(*b"TEXT", "hello").array(*b"VECS", &[[1.5f32; 3]]);
        let bytes = writer.finish();
        // Cast in place needs the alignment a memory map gives.
        let aligned: Vec<u128> = bytemuck::pod_collect_to_vec(&bytes);
        let bytes = &bytemuck::cast_slice::<u128, u8>(&aligned)[..bytes.len()];

        let container = Container::parse(bytes, *b"TEST", 3, "test").unwrap();
        assert_eq!(container.array::<u32>(*b"NUMS", "nums").unwrap(), &[1, 2, 3]);
        assert_eq!(container.text(*b"TEXT", "text").unwrap(), "hello");
        assert_eq!(container.array::<[f32; 3]>(*b"VECS", "vecs").unwrap(), &[[1.5; 3]]);
        assert!(container.raw(*b"NONE").is_none());
        assert!(matches!(Container::parse(bytes, *b"TEST", 4, "test"), Err(AssetError::Version { .. })));
        assert!(matches!(Container::parse(bytes, *b"OTHR", 3, "other"), Err(AssetError::WrongKind { .. })));
    }
}

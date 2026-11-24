//! Minimal pure-Rust stored-entry ZIP writer for `xtask_suite` test fixtures.

const EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const CD_SIG: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const LFH_SIG: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];

pub struct ZipWriter {
    entries: Vec<(String, Vec<u8>)>,
}

impl ZipWriter {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn add_file(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.entries.push((name.to_owned(), bytes));
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data) in &self.entries {
            let offset = u32::try_from(out.len()).map_err(|_| String::from("zip offset overflow"))?;
            let size = u32::try_from(data.len()).map_err(|_| String::from("zip size overflow"))?;
            let name_len = u16::try_from(name.len()).map_err(|_| String::from("zip name overflow"))?;
            let crc = crc32fast::hash(data);
            out.extend_from_slice(&LFH_SIG);
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            cd.extend_from_slice(&CD_SIG);
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&crc.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&name_len.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u32.to_le_bytes());
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_offset = u32::try_from(out.len()).map_err(|_| String::from("zip cd offset overflow"))?;
        let cd_size = u32::try_from(cd.len()).map_err(|_| String::from("zip cd size overflow"))?;
        let count = u16::try_from(self.entries.len()).map_err(|_| String::from("zip entry count overflow"))?;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&EOCD_SIG);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        Ok(out)
    }
}

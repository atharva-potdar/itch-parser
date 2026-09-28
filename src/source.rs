use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

pub trait PacketSource {
    fn next_packet(&mut self) -> std::io::Result<Option<Vec<u8>>>;
}

pub struct FilePacketSource {
    reader: BufReader<File>,
}

impl FilePacketSource {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self { reader: BufReader::new(file) })
    }
}

impl PacketSource for FilePacketSource {
    fn next_packet(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        let mut length_buf = [0u8; 4];

        match self.reader.read_exact(&mut length_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }

        let packet_length = u32::from_be_bytes(length_buf) as usize;
        let mut packet = vec![0u8; packet_length];
        self.reader.read_exact(&mut packet)?;

        Ok(Some(packet))
    }
}
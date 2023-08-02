use std::convert::TryInto;
use std::fmt::{Debug, Formatter};

/*
pub enum IoInt {
    Byte(u8, [u8; 1]),
    Word(u16, [u8; 2]),
    DWord(u32, [u8; 4]),
    QWord(u64, [u8; 8]),
    Data(Vec<u8>),
}

impl IoInt {
    pub fn new_byte(n: u8) -> Self {
        Self::Byte(n, [n])
    }
    pub fn new_word(n: u16) -> Self {
        Self::Word(n, n.to_le_bytes())
    }
    pub fn new_dword(n: u32) -> Self {
        Self::DWord(n, n.to_le_bytes())
    }
    pub fn new_qword(n: u64) -> Self {
        Self::QWord(n, n.to_le_bytes())
    }

}

impl From<&[u8]> for IoInt {
    fn from(bytes: &[u8]) -> Self {
        match bytes.len() {
            1 => Self::Byte(bytes[0], [bytes[0]]),
            2 => {
                let n = u16::from_le_bytes(bytes.try_into().unwrap());
                Self::Word(n, n.to_le_bytes())
            },
            4 => {
                let n = u32::from_le_bytes(bytes.try_into().unwrap());
                Self::DWord(n, n.to_le_bytes())
            },
            8 => {
                let n = u64::from_le_bytes(bytes.try_into().unwrap());
                Self::QWord(n, n.to_le_bytes())
            },
            _ => Self::Data(bytes.to_vec()),
        }
    }
}

 */

pub enum WriteableInt {
    Byte(u8),
    Word(u16),
    DWord(u32),
    QWord(u64),
    Data(Vec<u8>),
}

impl From<&[u8]> for WriteableInt {
    fn from(bytes: &[u8]) -> Self {
        match bytes.len() {
            1 => Self::Byte(bytes[0]),
            2 => Self::Word(u16::from_le_bytes(bytes.try_into().unwrap())),
            4 => Self::DWord(u32::from_le_bytes(bytes.try_into().unwrap())),
            8 => Self::QWord(u64::from_le_bytes(bytes.try_into().unwrap())),
            _ => Self::Data(bytes.to_vec()),
        }
    }
}

pub enum ReadableInt {
    Byte(u8, [u8; 1]),
    Word(u16, [u8; 2]),
    DWord(u32, [u8; 4]),
}

impl ReadableInt {

    pub fn new_byte(n: u8) -> Self {
        Self::Byte(n, [n])
    }
    pub fn new_word(n: u16) -> Self {
        Self::Word(n, n.to_le_bytes())
    }
    pub fn new_dword(n: u32) -> Self {
        Self::DWord(n, n.to_le_bytes())
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            ReadableInt::Byte(_, bs) => bs,
            ReadableInt::Word(_, bs) => bs,
            ReadableInt::DWord(_, bs) => bs,
        }
    }

    pub fn read(&self, buffer: &mut [u8]) {
        let bs = self.as_bytes();
        if buffer.len() >= bs.len() {
            buffer[..bs.len()].copy_from_slice(bs);
        }
    }
}

impl From<u8> for ReadableInt {
    fn from(value: u8) -> Self {
        ReadableInt::new_byte(value)
    }
}

impl From<u16> for ReadableInt {
    fn from(value: u16) -> Self {
        ReadableInt::new_word(value)
    }
}
impl From<u32> for ReadableInt {
    fn from(value: u32) -> Self {
        ReadableInt::new_dword(value)
    }
}

impl Debug for ReadableInt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadableInt::Byte(n, _) => write!(f, "Byte({})", n),
            ReadableInt::Word(n, _) => write!(f, "Word({})", n),
            ReadableInt::DWord(n, _) => write!(f, "DWord({})", n),
        }
    }
}

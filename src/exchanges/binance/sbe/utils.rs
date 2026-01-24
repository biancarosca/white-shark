use crate::error::{Error, Result};
use zerocopy::{FromBytes, Ref, Unaligned};
use zerocopy::byteorder::{LittleEndian, I64, U16, U32};

pub struct SbeCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SbeCursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn skip(&mut self, len: usize) -> Result<()> {
        self.advance(len)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        if self.remaining() < 1 {
            return Err(Error::SbeDecode("Not enough data to read u8".into()));
        }
        let value = self.data[self.pos];
        self.pos += 1;
        Ok(value)
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_u16_le(&mut self) -> Result<u16> {
        Ok(self.read_zerocopy::<U16<LittleEndian>>()?.get())
    }

    pub fn read_u32_le(&mut self) -> Result<u32> {
        Ok(self.read_zerocopy::<U32<LittleEndian>>()?.get())
    }

    pub fn read_i64_le(&mut self) -> Result<i64> {
        Ok(self.read_zerocopy::<I64<LittleEndian>>()?.get())
    }

    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.remaining() < len {
            return Err(Error::SbeDecode(format!(
                "Not enough data to read bytes: need {} bytes, have {} bytes",
                len,
                self.remaining()
            )));
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.data[start..start + len])
    }

    pub fn read_var_string8(&mut self) -> Result<&'a str> {
        let length = self.read_u8()? as usize;
        let bytes = self.read_bytes(length)?;
        std::str::from_utf8(bytes)
            .map_err(|e| Error::SbeDecode(format!("Invalid UTF-8 in symbol: {}", e)))
    }

    fn remaining_slice(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    fn advance(&mut self, len: usize) -> Result<()> {
        if self.remaining() < len {
            return Err(Error::SbeDecode(format!(
                "Not enough data to skip: need {} bytes, have {} bytes",
                len,
                self.remaining()
            )));
        }
        self.pos += len;
        Ok(())
    }

    fn read_zerocopy<T: FromBytes + Unaligned>(&mut self) -> Result<&'a T> {
        let slice = self.remaining_slice();
        let (value, rest) = Ref::<_, T>::new_from_prefix(slice)
            .ok_or_else(|| Error::SbeDecode("Not enough data to read value".into()))?;
        self.pos = self.data.len() - rest.len();
        Ok(value.into_ref())
    }
}

pub fn read_group_size(cursor: &mut SbeCursor<'_>) -> Result<(u16, u32)> {
    let block_length = cursor.read_u16_le()?;
    let num_in_group = cursor.read_u32_le()?;
    Ok((block_length, num_in_group))
}

pub fn read_group_size16(cursor: &mut SbeCursor<'_>) -> Result<(u16, u16)> {
    let block_length = cursor.read_u16_le()?;
    let num_in_group = cursor.read_u16_le()?;
    Ok((block_length, num_in_group))
}

pub fn read_i64_le_from(data: &[u8]) -> Result<i64> {
    let (value, _) = Ref::<_, I64<LittleEndian>>::new_from_prefix(data)
        .ok_or_else(|| Error::SbeDecode("Not enough data to read i64".into()))?;
    Ok(value.get())
}

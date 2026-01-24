use std::io::{Cursor, Read};
use crate::error::{Error, Result};
use byteorder::{LittleEndian, ReadBytesExt};

pub fn read_var_string8(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let position = cursor.position() as usize;
    let data = cursor.get_ref();
    
    if position >= data.len() {
        return Err(Error::SbeDecode("Not enough data to read string length".into()));
    }
    
    let length = data[position] as usize;
    
    if position + 1 + length > data.len() {
        return Err(Error::SbeDecode(format!(
            "Not enough data to read string: need {} bytes, have {} bytes",
            length + 1,
            data.len() - position
        )));
    }
    
    cursor.read_u8()?;
    let mut bytes = vec![0u8; length];
    cursor.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|e| Error::SbeDecode(format!("Invalid UTF-8 in symbol: {}", e)))
}

pub fn read_group_size(cursor: &mut Cursor<&[u8]>) -> Result<(u16, u32)> {
    let block_length = cursor.read_u16::<LittleEndian>()?;
    let num_in_group = cursor.read_u32::<LittleEndian>()?;
    Ok((block_length, num_in_group))
}

pub fn read_group_size16(cursor: &mut Cursor<&[u8]>) -> Result<(u16, u16)> {
    let block_length = cursor.read_u16::<LittleEndian>()?;
    let num_in_group = cursor.read_u16::<LittleEndian>()?;
    Ok((block_length, num_in_group))
}

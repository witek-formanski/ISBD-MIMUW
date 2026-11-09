use std::io::{self, ErrorKind, Read, Write};

use lz4_flex::block::{compress, decompress};

use crate::batch::{BATCH_SIZE, Batch, Column, ColumnData, DataType, Field, validate_fields};

const MAGIC: &[u8; 4] = b"ISBD";
const VERSION: u16 = 1;

pub struct FileWriter<W: Write> {
    writer: W,
    schema: Vec<Field>,
}

impl<W: Write> FileWriter<W> {
    pub fn new(mut writer: W, schema: &[Field]) -> io::Result<Self> {
        validate_fields(schema)?;
        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;
        write_varint(&mut writer, usize_to_u64(schema.len())?)?;
        for field in schema {
            let tag = match field.data_type {
                DataType::Int64 => 1,
                DataType::Boolean => 2,
                DataType::Varchar => 3,
            };
            writer.write_all(&[tag])?;
            write_varint(&mut writer, usize_to_u64(field.name.len())?)?;
            writer.write_all(field.name.as_bytes())?;
        }
        Ok(Self {
            writer,
            schema: schema.to_vec(),
        })
    }

    pub fn write_batch(&mut self, batch: &Batch) -> io::Result<()> {
        if batch.schema() != self.schema {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "batch schema does not match file schema",
            ));
        }
        self.writer
            .write_all(&(batch.row_count() as u32).to_le_bytes())?;
        for column in batch.columns() {
            let payload = match &column.data {
                ColumnData::Int64(values) => encode_int64(values),
                ColumnData::Boolean(values) => encode_boolean(values),
                ColumnData::Varchar(values) => encode_varchar(values)?,
            };
            write_varint(&mut self.writer, usize_to_u64(payload.len())?)?;
            self.writer.write_all(&payload)?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.writer.flush()?;
        Ok(self.writer)
    }
}

pub struct FileReader<R: Read> {
    reader: R,
    schema: Vec<Field>,
    finished: bool,
}

impl<R: Read> FileReader<R> {
    pub fn new(mut reader: R) -> io::Result<Self> {
        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid_data("invalid file magic"));
        }
        let mut version = [0; 2];
        reader.read_exact(&mut version)?;
        if u16::from_le_bytes(version) != VERSION {
            return Err(invalid_data("unsupported file version"));
        }
        let field_count = u64_to_usize(read_varint(&mut reader)?)?;
        let mut schema = Vec::new();
        for _ in 0..field_count {
            let mut tag = [0];
            reader.read_exact(&mut tag)?;
            let data_type = match tag[0] {
                1 => DataType::Int64,
                2 => DataType::Boolean,
                3 => DataType::Varchar,
                _ => return Err(invalid_data("unknown column type")),
            };
            let name_len = read_varint(&mut reader)?;
            let name = String::from_utf8(read_bytes(&mut reader, name_len)?)
                .map_err(|_| invalid_data("column name is not UTF-8"))?;
            schema.push(Field { name, data_type });
        }
        validate_fields(&schema).map_err(|error| invalid_data(&error.to_string()))?;
        Ok(Self {
            reader,
            schema,
            finished: false,
        })
    }

    pub fn schema(&self) -> &[Field] {
        &self.schema
    }

    pub fn next_batch(&mut self) -> io::Result<Option<Batch>> {
        if self.finished {
            return Ok(None);
        }
        let mut first = [0];
        let bytes_read = loop {
            match self.reader.read(&mut first) {
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        if bytes_read == 0 {
            self.finished = true;
            return Ok(None);
        }
        let mut count_bytes = [first[0], 0, 0, 0];
        self.reader.read_exact(&mut count_bytes[1..])?;
        let row_count = u32::from_le_bytes(count_bytes) as usize;
        if row_count > BATCH_SIZE {
            return Err(invalid_data("batch exceeds BATCH_SIZE"));
        }
        let mut columns = Vec::with_capacity(self.schema.len());
        for field in &self.schema {
            let payload_len = read_varint(&mut self.reader)?;
            let payload = read_bytes(&mut self.reader, payload_len)?;
            let data = match field.data_type {
                DataType::Int64 => ColumnData::Int64(decode_int64(&payload, row_count)?),
                DataType::Boolean => ColumnData::Boolean(decode_boolean(&payload, row_count)?),
                DataType::Varchar => ColumnData::Varchar(decode_varchar(&payload, row_count)?),
            };
            columns.push(Column::new(&field.name, data));
        }
        Batch::with_row_count(columns, row_count)
            .map(Some)
            .map_err(|error| invalid_data(&error.to_string()))
    }
}

fn encode_int64(values: &[i64]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut previous = 0i128;
    for &value in values {
        let current = i128::from(value);
        let delta = current - previous;
        let zigzag = if delta >= 0 {
            (delta as u128) << 1
        } else {
            ((-delta as u128) << 1) - 1
        };
        write_varint_u128(&mut output, zigzag);
        previous = current;
    }
    output
}

fn decode_int64(input: &[u8], count: usize) -> io::Result<Vec<i64>> {
    let mut cursor = 0;
    let mut previous = 0i128;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let zigzag = read_varint_u128(input, &mut cursor)?;
        if zigzag > ((1u128 << 65) - 2) {
            return Err(invalid_data("INT64 delta out of range"));
        }
        let delta = if zigzag & 1 == 0 {
            (zigzag >> 1) as i128
        } else {
            -((zigzag >> 1) as i128) - 1
        };
        let current = previous + delta;
        values.push(i64::try_from(current).map_err(|_| invalid_data("INT64 value out of range"))?);
        previous = current;
    }
    if cursor != input.len() {
        return Err(invalid_data("extra bytes in INT64 column"));
    }
    Ok(values)
}

fn encode_boolean(values: &[u8]) -> Vec<u8> {
    let mut output = vec![0; values.len().div_ceil(8)];
    for (index, &value) in values.iter().enumerate() {
        output[index / 8] |= value << (index % 8);
    }
    output
}

fn decode_boolean(input: &[u8], count: usize) -> io::Result<Vec<u8>> {
    if input.len() != count.div_ceil(8) {
        return Err(invalid_data("invalid BOOLEAN column length"));
    }
    if !count.is_multiple_of(8) && input.last().is_some_and(|byte| byte >> (count % 8) != 0) {
        return Err(invalid_data("nonzero unused BOOLEAN bits"));
    }
    Ok((0..count)
        .map(|index| (input[index / 8] >> (index % 8)) & 1)
        .collect())
}

fn encode_varchar(values: &[String]) -> io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    for value in values {
        write_varint(&mut raw, usize_to_u64(value.len())?)?;
        raw.extend_from_slice(value.as_bytes());
    }
    let compressed = compress(&raw);
    let mut payload = Vec::new();
    write_varint(&mut payload, usize_to_u64(raw.len())?)?;
    payload.extend_from_slice(&compressed);
    Ok(payload)
}

fn decode_varchar(payload: &[u8], count: usize) -> io::Result<Vec<String>> {
    let mut cursor = 0;
    let raw_len = u64_to_usize(
        read_varint_u128(payload, &mut cursor)?
            .try_into()
            .map_err(|_| invalid_data("VARCHAR length out of range"))?,
    )?;
    let raw = decompress(&payload[cursor..], raw_len)
        .map_err(|error| invalid_data(&format!("invalid LZ4 block: {error}")))?;
    if raw.len() != raw_len {
        return Err(invalid_data("invalid VARCHAR data length"));
    }
    let mut values = Vec::with_capacity(count);
    let mut cursor = 0;
    for _ in 0..count {
        let length = u64_to_usize(
            read_varint_u128(&raw, &mut cursor)?
                .try_into()
                .map_err(|_| invalid_data("string length out of range"))?,
        )?;
        let end = cursor
            .checked_add(length)
            .filter(|&end| end <= raw.len())
            .ok_or_else(|| invalid_data("string exceeds VARCHAR block"))?;
        values.push(
            String::from_utf8(raw[cursor..end].to_vec())
                .map_err(|_| invalid_data("string is not UTF-8"))?,
        );
        cursor = end;
    }
    if cursor != raw.len() {
        return Err(invalid_data("extra bytes in VARCHAR column"));
    }
    Ok(values)
}

fn write_varint<W: Write>(writer: &mut W, value: u64) -> io::Result<()> {
    let mut encoded = Vec::new();
    write_varint_u128(&mut encoded, value as u128);
    writer.write_all(&encoded)
}

fn write_varint_u128(output: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn read_varint<R: Read>(reader: &mut R) -> io::Result<u64> {
    let mut result = 0u64;
    for index in 0..10 {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        let part = (byte[0] & 0x7f) as u64;
        if index == 9 && part > 1 {
            return Err(invalid_data("VLE value exceeds u64"));
        }
        result |= part << (index * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(invalid_data("VLE value is too long"))
}

fn read_varint_u128(input: &[u8], cursor: &mut usize) -> io::Result<u128> {
    let mut result = 0u128;
    for index in 0..19 {
        let byte = *input
            .get(*cursor)
            .ok_or_else(|| invalid_data("truncated VLE value"))?;
        *cursor += 1;
        let part = (byte & 0x7f) as u128;
        if index == 18 && part > 3 {
            return Err(invalid_data("VLE value exceeds u128"));
        }
        result |= part << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(invalid_data("VLE value is too long"))
}

fn read_bytes<R: Read>(reader: &mut R, length: u64) -> io::Result<Vec<u8>> {
    let length = u64_to_usize(length)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| invalid_data("block is too large"))?;
    bytes.resize(length, 0);
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn usize_to_u64(value: usize) -> io::Result<u64> {
    u64::try_from(value).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "length exceeds u64"))
}

fn u64_to_usize(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid_data("length exceeds usize"))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_int64_bytes() {
        assert_eq!(encode_int64(&[0, 1, -1, 3]), [0, 2, 3, 8]);
        assert_eq!(decode_int64(&[0, 2, 3, 8], 4).unwrap(), [0, 1, -1, 3]);
        assert_eq!(
            decode_int64(&[0xff, 0xff], 1).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn known_boolean_bits() {
        let bits = [1, 0, 1, 1, 0, 0, 0, 1, 1];
        assert_eq!(encode_boolean(&bits), [0x8d, 0x01]);
        assert_eq!(decode_boolean(&[0x8d, 0x01], 9).unwrap(), bits);
        assert!(decode_boolean(&[0x8d, 0x81], 9).is_err());
    }
}

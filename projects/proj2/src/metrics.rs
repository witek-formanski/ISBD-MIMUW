use std::io::{self, ErrorKind};

use crate::{Batch, ColumnData, DataType, Field};

#[derive(Debug)]
enum State {
    Sum(i128),
    TrueCount(u64),
    AsciiCount(u64),
}

#[derive(Debug)]
pub struct Metrics {
    schema: Vec<Field>,
    states: Vec<State>,
    rows: u64,
}

impl Metrics {
    pub fn new(schema: &[Field]) -> Self {
        let states = schema
            .iter()
            .map(|field| match field.data_type {
                DataType::Int64 => State::Sum(0),
                DataType::Boolean => State::TrueCount(0),
                DataType::Varchar => State::AsciiCount(0),
            })
            .collect();
        Self {
            schema: schema.to_vec(),
            states,
            rows: 0,
        }
    }

    pub fn update(&mut self, batch: &Batch) -> io::Result<()> {
        if batch.schema() != self.schema {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "batch schema does not match metrics schema",
            ));
        }
        let batch_rows = u64::try_from(batch.row_count())
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "too many rows"))?;
        let new_rows = self
            .rows
            .checked_add(batch_rows)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "row count overflow"))?;
        for (column, state) in batch.columns().iter().zip(&mut self.states) {
            match (&column.data, state) {
                (ColumnData::Int64(values), State::Sum(sum)) => {
                    for &value in values {
                        *sum = sum.checked_add(i128::from(value)).ok_or_else(|| {
                            io::Error::new(ErrorKind::InvalidInput, "INT64 sum overflow")
                        })?;
                    }
                }
                (ColumnData::Boolean(values), State::TrueCount(true_count)) => {
                    let count = values.iter().map(|&value| value as u64).sum::<u64>();
                    *true_count = true_count.checked_add(count).ok_or_else(|| {
                        io::Error::new(ErrorKind::InvalidInput, "BOOLEAN count overflow")
                    })?;
                }
                (ColumnData::Varchar(values), State::AsciiCount(ascii_count)) => {
                    for value in values {
                        let count = value
                            .as_bytes()
                            .iter()
                            .filter(|byte| byte.is_ascii())
                            .count();
                        *ascii_count = ascii_count.checked_add(count as u64).ok_or_else(|| {
                            io::Error::new(ErrorKind::InvalidInput, "ASCII count overflow")
                        })?;
                    }
                }
                _ => unreachable!("batch schema was validated above"),
            }
        }
        self.rows = new_rows;
        Ok(())
    }

    pub fn row_count(&self) -> u64 {
        self.rows
    }

    pub fn int64_mean(&self, index: usize) -> Option<f64> {
        match self.states.get(index)? {
            State::Sum(sum) if self.rows > 0 => Some(*sum as f64 / self.rows as f64),
            _ => None,
        }
    }

    pub fn boolean_entropy(&self, index: usize) -> Option<f64> {
        let State::TrueCount(true_count) = self.states.get(index)? else {
            return None;
        };
        if self.rows == 0 {
            return None;
        }
        let probability = *true_count as f64 / self.rows as f64;
        Some(if probability == 0.0 || probability == 1.0 {
            0.0
        } else {
            -probability * probability.log2() - (1.0 - probability) * (1.0 - probability).log2()
        })
    }

    pub fn ascii_count(&self, index: usize) -> Option<u64> {
        match self.states.get(index)? {
            State::AsciiCount(count) => Some(*count),
            _ => None,
        }
    }

    pub fn report(&self) -> String {
        let mut output = format!("Rows: {}\n", self.rows);
        for (index, field) in self.schema.iter().enumerate() {
            match field.data_type {
                DataType::Int64 => match self.int64_mean(index) {
                    Some(mean) => {
                        output.push_str(&format!("{} (INT64): mean = {mean:.6}\n", field.name))
                    }
                    None => output.push_str(&format!("{} (INT64): mean = no data\n", field.name)),
                },
                DataType::Boolean => match self.boolean_entropy(index) {
                    Some(entropy) => output.push_str(&format!(
                        "{} (BOOLEAN): binary entropy = {entropy:.6}\n",
                        field.name
                    )),
                    None => output.push_str(&format!(
                        "{} (BOOLEAN): binary entropy = no data\n",
                        field.name
                    )),
                },
                DataType::Varchar => output.push_str(&format!(
                    "{} (VARCHAR): ASCII character count = {}\n",
                    field.name,
                    self.ascii_count(index).expect("VARCHAR column")
                )),
            }
        }
        output
    }
}

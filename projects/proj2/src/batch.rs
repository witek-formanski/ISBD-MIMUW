use std::collections::HashSet;
use std::io::{self, ErrorKind};

pub const BATCH_SIZE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataType {
    Int64,
    Boolean,
    Varchar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub data_type: DataType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnData {
    Int64(Vec<i64>),
    Boolean(Vec<u8>),
    Varchar(Vec<String>),
}

impl ColumnData {
    pub fn len(&self) -> usize {
        match self {
            Self::Int64(values) => values.len(),
            Self::Boolean(values) => values.len(),
            Self::Varchar(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn data_type(&self) -> DataType {
        match self {
            Self::Int64(_) => DataType::Int64,
            Self::Boolean(_) => DataType::Boolean,
            Self::Varchar(_) => DataType::Varchar,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data: ColumnData,
}

impl Column {
    pub fn new(name: impl Into<String>, data: ColumnData) -> Self {
        Self {
            name: name.into(),
            data,
        }
    }

    pub fn field(&self) -> Field {
        Field {
            name: self.name.clone(),
            data_type: self.data.data_type(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellRef<'a> {
    Int64(i64),
    Boolean(bool),
    Varchar(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    columns: Vec<Column>,
    row_count: usize,
}

impl Batch {
    pub fn new(columns: Vec<Column>) -> io::Result<Self> {
        let row_count = columns.first().map_or(0, |column| column.data.len());
        Self::with_row_count(columns, row_count)
    }

    pub fn with_row_count(columns: Vec<Column>, row_count: usize) -> io::Result<Self> {
        if row_count > BATCH_SIZE {
            return Err(invalid_input("batch exceeds BATCH_SIZE"));
        }
        validate_fields(&columns.iter().map(Column::field).collect::<Vec<_>>())?;
        for column in &columns {
            if column.data.len() != row_count {
                return Err(invalid_input("columns have different row counts"));
            }
            if let ColumnData::Boolean(values) = &column.data
                && values.iter().any(|&value| value > 1)
            {
                return Err(invalid_input("BOOLEAN values must be 0 or 1"));
            }
        }
        Ok(Self { columns, row_count })
    }

    pub fn row_count(&self) -> usize {
        self.row_count
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn schema(&self) -> Vec<Field> {
        self.columns.iter().map(Column::field).collect()
    }

    pub fn cell(&self, column_index: usize, row_index: usize) -> Option<CellRef<'_>> {
        if row_index >= self.row_count {
            return None;
        }
        match &self.columns.get(column_index)?.data {
            ColumnData::Int64(values) => Some(CellRef::Int64(values[row_index])),
            ColumnData::Boolean(values) => Some(CellRef::Boolean(values[row_index] == 1)),
            ColumnData::Varchar(values) => Some(CellRef::Varchar(&values[row_index])),
        }
    }

    pub fn cell_named(&self, name: &str, row_index: usize) -> Option<CellRef<'_>> {
        self.columns
            .iter()
            .position(|column| column.name == name)
            .and_then(|index| self.cell(index, row_index))
    }

    pub fn concat_varchar(&mut self, target: &str, left: &str, right: &str) -> io::Result<()> {
        if target.is_empty() {
            return Err(invalid_input("target column name cannot be empty"));
        }
        if self.columns.iter().any(|column| column.name == target) {
            return Err(invalid_input("target column already exists"));
        }
        let get_strings = |name: &str| -> io::Result<&[String]> {
            match self.columns.iter().find(|column| column.name == name) {
                Some(Column {
                    data: ColumnData::Varchar(values),
                    ..
                }) => Ok(values),
                Some(_) => Err(invalid_input("source column is not VARCHAR")),
                None => Err(invalid_input("source column not found")),
            }
        };
        let left_values = get_strings(left)?;
        let right_values = get_strings(right)?;
        let values = left_values
            .iter()
            .zip(right_values)
            .map(|(left, right)| format!("{left}{right}"))
            .collect();
        self.columns
            .push(Column::new(target, ColumnData::Varchar(values)));
        Ok(())
    }
}

pub(crate) fn validate_fields(fields: &[Field]) -> io::Result<()> {
    let mut names = HashSet::new();
    for field in fields {
        if field.name.is_empty() || !names.insert(&field.name) {
            return Err(invalid_input("column names must be nonempty and unique"));
        }
    }
    Ok(())
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message)
}

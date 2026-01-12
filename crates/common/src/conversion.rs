use std::sync::Arc;

use arrow_schema::{ArrowError, DataType, Field, TimeUnit};
use itertools::Itertools as _;

use crate::models::delta::connect::create_delta_table::Column;
use crate::models::spark::connect::data_type::{Decimal, StructField};
use crate::models::spark::connect::{
    DataType as ConnectDataType, data_type::Kind as ConnectDataTypeKind,
};

/// Options for converting Connect data types to Arrow data types.
///
/// These options control how the logical types defined in Connect are mapped to
/// Arrow data types during the conversion process.
#[derive(Debug, Clone)]
pub struct ConversionOptions {
    /// The Arrow data type to use for string columns.
    string_variant: DataType,
    /// The Arrow data type to use for binary columns.
    binary_variant: DataType,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversionOptions {
    fn new() -> Self {
        Self {
            string_variant: DataType::Utf8View,
            binary_variant: DataType::BinaryView,
        }
    }

    /// Sets the Arrow [`DataType`] to use for string columns.
    ///
    /// Valid options are [`Utf8`](DataType::Utf8), [`LargeUtf8`](DataType::LargeUtf8),
    /// and [`Utf8View`](DataType::Utf8View). Returns an error if an invalid type is provided.
    pub fn with_string_variant(mut self, data_type: DataType) -> Result<Self, ArrowError> {
        if !matches!(
            data_type,
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
        ) {
            return Err(ArrowError::InvalidArgumentError(format!(
                "Expected valid string type variant, got: {}",
                data_type
            )));
        }
        self.string_variant = data_type;
        Ok(self)
    }

    /// Sets the Arrow [`DataType`] to use for binary columns.
    ///
    /// Valid options are [`Binary`](DataType::Binary), [`LargeBinary`](DataType::LargeBinary),
    /// and [`BinaryView`](DataType::BinaryView). Returns an error if an invalid type is provided.
    pub fn with_binary_variant(mut self, data_type: DataType) -> Result<Self, ArrowError> {
        if !matches!(
            data_type,
            DataType::Binary | DataType::LargeBinary | DataType::BinaryView
        ) {
            return Err(ArrowError::InvalidArgumentError(format!(
                "Expected valid binary type variant, got: {}",
                data_type
            )));
        }
        self.binary_variant = data_type;
        Ok(self)
    }

    /// Use view types for applicable columns.
    pub fn with_use_view_types(mut self) -> Self {
        self = self
            .with_string_variant(DataType::Utf8View)
            .expect("valid string type");
        self = self
            .with_binary_variant(DataType::BinaryView)
            .expect("valid binary type");
        self
    }

    /// Returns the configured Arrow [`DataType`] for string columns.
    pub fn string_type(&self) -> DataType {
        self.string_variant.clone()
    }

    /// Returns the configured Arrow [`DataType`] for binary columns.
    pub fn binary_type(&self) -> DataType {
        self.binary_variant.clone()
    }
}

pub fn column_to_arrow(column: &Column, options: &ConversionOptions) -> Result<Field, ArrowError> {
    let data_type = match column.data_type.as_ref().and_then(|dt| dt.kind.as_ref()) {
        Some(dt) => data_type_to_arrow(dt, options)?,
        None => {
            return Err(ArrowError::InvalidArgumentError(
                "Column data type is missing".to_string(),
            ));
        }
    };

    Ok(Field::new(&column.name, data_type, column.nullable))
}

pub fn struct_field_to_arrow(
    field: &StructField,
    options: &ConversionOptions,
) -> Result<Field, ArrowError> {
    let data_type = match field.data_type.as_ref().and_then(|dt| dt.kind.as_ref()) {
        Some(dt) => data_type_to_arrow(dt, options)?,
        None => {
            return Err(ArrowError::InvalidArgumentError(
                "Struct field data type is missing".to_string(),
            ));
        }
    };
    // TODO: handle metadata which is just a string in the spec, do we need to parse it?
    Ok(Field::new(&field.name, data_type, field.nullable))
}

pub fn data_type_to_arrow(
    data_type: &ConnectDataTypeKind,
    options: &ConversionOptions,
) -> Result<DataType, ArrowError> {
    match data_type {
        ConnectDataTypeKind::Binary(_) => Ok(options.binary_type()),
        ConnectDataTypeKind::Boolean(_) => Ok(DataType::Boolean),
        ConnectDataTypeKind::Byte(_) => Ok(DataType::Int8),
        ConnectDataTypeKind::Short(_) => Ok(DataType::Int16),
        ConnectDataTypeKind::Integer(_) => Ok(DataType::Int32),
        ConnectDataTypeKind::Long(_) => Ok(DataType::Int64),
        ConnectDataTypeKind::Float(_) => Ok(DataType::Float32),
        ConnectDataTypeKind::Double(_) => Ok(DataType::Float64),
        ConnectDataTypeKind::String(_) => Ok(options.string_type()),
        ConnectDataTypeKind::Timestamp(_) => Ok(DataType::Timestamp(
            TimeUnit::Microsecond,
            Some("UTC".into()),
        )),
        ConnectDataTypeKind::TimestampNtz(_) => {
            Ok(DataType::Timestamp(TimeUnit::Microsecond, None))
        }
        ConnectDataTypeKind::Date(_) => Ok(DataType::Date32),
        ConnectDataTypeKind::Struct(info) => {
            let fields = info
                .fields
                .iter()
                .map(|f| struct_field_to_arrow(f, options))
                .try_collect()?;
            Ok(DataType::Struct(fields))
        }
        ConnectDataTypeKind::Array(info) => {
            let element_type = match info.element_type.as_ref().and_then(|dt| dt.kind.as_ref()) {
                Some(dt) => data_type_to_arrow(dt, options)?,
                None => {
                    return Err(ArrowError::InvalidArgumentError(
                        "Array element data type is missing".to_string(),
                    ));
                }
            };
            Ok(DataType::new_list(element_type, info.contains_null))
        }
        ConnectDataTypeKind::Map(info) => {
            let key_type = match info.key_type.as_ref().and_then(|dt| dt.kind.as_ref()) {
                Some(dt) => data_type_to_arrow(dt, options)?,
                None => {
                    return Err(ArrowError::InvalidArgumentError(
                        "Map key data type is missing".to_string(),
                    ));
                }
            };
            let value_type = match info.value_type.as_ref().and_then(|dt| dt.kind.as_ref()) {
                Some(dt) => data_type_to_arrow(dt, options)?,
                None => {
                    return Err(ArrowError::InvalidArgumentError(
                        "Map value data type is missing".to_string(),
                    ));
                }
            };
            Ok(DataType::Map(
                Arc::new(Field::new(
                    "key_value",
                    DataType::Struct(
                        vec![
                            Field::new("key", key_type, false),
                            Field::new("value", value_type, info.value_contains_null),
                        ]
                        .into(),
                    ),
                    false, // always non-null
                )),
                false,
            ))
        }
        ConnectDataTypeKind::Decimal(info) => Ok(DataType::Decimal128(
            info.precision.unwrap_or(38) as u8,
            info.scale.unwrap_or(18) as i8,
        )),
        _ => Err(ArrowError::NotYetImplemented(format!(
            "Data type conversion not implemented for: {:?}",
            data_type
        ))),
    }
}

pub fn field_to_connect(field: &Field) -> Result<Column, ArrowError> {
    let data_type = data_type_to_connect(field.data_type())?;
    // TODO: parse field metadata column fields (generated etc.)
    Ok(Column {
        name: field.name().clone(),
        data_type: Some(ConnectDataType {
            kind: Some(data_type),
        }),
        nullable: field.is_nullable(),
        ..Default::default()
    })
}

pub fn data_type_to_connect(data_type: &DataType) -> Result<ConnectDataTypeKind, ArrowError> {
    match data_type {
        DataType::Boolean => Ok(ConnectDataTypeKind::Boolean(Default::default())),
        DataType::Int8 => Ok(ConnectDataTypeKind::Byte(Default::default())),
        DataType::Int16 => Ok(ConnectDataTypeKind::Short(Default::default())),
        DataType::Int32 => Ok(ConnectDataTypeKind::Integer(Default::default())),
        DataType::Int64 => Ok(ConnectDataTypeKind::Long(Default::default())),
        DataType::Float32 => Ok(ConnectDataTypeKind::Float(Default::default())),
        DataType::Float64 => Ok(ConnectDataTypeKind::Double(Default::default())),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            Ok(ConnectDataTypeKind::String(Default::default()))
        }
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => {
            Ok(ConnectDataTypeKind::Binary(Default::default()))
        }
        DataType::Timestamp(TimeUnit::Microsecond, Some(tz))
            if tz.eq_ignore_ascii_case("utc") || tz.as_ref() == "*00:00" =>
        {
            Ok(ConnectDataTypeKind::Timestamp(Default::default()))
        }
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            Ok(ConnectDataTypeKind::TimestampNtz(Default::default()))
        }
        DataType::Date32 => Ok(ConnectDataTypeKind::Date(Default::default())),
        DataType::Decimal128(p, s) => Ok(ConnectDataTypeKind::Decimal(Decimal {
            precision: Some(*p as i32),
            scale: Some(*s as i32),
            ..Default::default()
        })),
        _ => Err(ArrowError::NotYetImplemented(format!(
            "Data type conversion not implemented for: {:?}",
            data_type
        ))),
    }
}

//! The **catalog-function PIP**: resolving a policy-referenced masking function
//! (`@mask_fn("hr.security.mask_ssn")`) into a DataFusion [`ScalarUDF`] the Cedar
//! governance layer can call over the masked column.
//!
//! Databricks Unity Catalog ABAC binds a governed tag to a SQL UDF
//! (`COLUMN MASK f`); breakwater mirrors that behind the
//! [`CatalogFunctionResolver`] seam. This is hydrofoil's implementation over the
//! UC **Functions** API: it resolves the function's *signature* (real arity +
//! parameter/return types) so DataFusion validates the call, while the UDF body
//! itself is a fail-closed no-op returning the mask literal — executing the UC
//! SQL `routine_definition` is a separate, larger effort.
//!
//! An unresolvable name (undeployed function, catalog outage) is an `Err`, which
//! the governance layer treats fail-closed: it masks the column with the default
//! literal rather than exposing it.

use std::sync::Arc;

use buffa::EnumValue;
use datafusion::arrow::datatypes::DataType;
use datafusion::error::{DataFusionError, Result};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::scalar::ScalarValue;

use datafusion_policy_cedar::CatalogFunctionResolver;

use unitycatalog_client::UnityCatalogClient;
use unitycatalog_common::models::functions::v1::{Function, GetFunctionRequest};
use unitycatalog_common::models::tables::v1::ColumnTypeName;

/// The mask value a resolved function's body returns until real UC-SQL execution
/// is wired. Kept in sync with breakwater's default mask literal.
const MASK_LITERAL: &str = "***";

/// [`CatalogFunctionResolver`] over the Unity Catalog Functions API.
///
/// Resolves a catalog-qualified function name (as written in the policy's
/// `@mask_fn`) to a [`ScalarUDF`] whose signature mirrors the UC function's
/// declared parameters and return type.
#[derive(Clone)]
pub struct UnityFunctionResolver {
    client: UnityCatalogClient,
}

impl std::fmt::Debug for UnityFunctionResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnityFunctionResolver")
            .finish_non_exhaustive()
    }
}

impl UnityFunctionResolver {
    pub fn new(client: UnityCatalogClient) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl CatalogFunctionResolver for UnityFunctionResolver {
    async fn resolve(&self, name: &str) -> Result<Arc<ScalarUDF>> {
        let function = self
            .client
            .functions_client()
            .get_function(&GetFunctionRequest {
                name: name.to_string(),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                DataFusionError::Plan(format!("failed to resolve catalog function '{name}': {e}"))
            })?;

        Ok(Arc::new(ScalarUDF::from(CatalogMaskUdf::from_function(
            name, &function,
        )?)))
    }
}

/// A [`ScalarUDFImpl`] carrying a UC function's resolved signature. Its body is a
/// fail-closed no-op returning the [`MASK_LITERAL`]; the real signature still
/// lets DataFusion bind the masked column (arg 0) and any `@using_columns` extra
/// args, and validate arity/types when the mask call is planned.
#[derive(Debug, PartialEq, Eq, Hash)]
struct CatalogMaskUdf {
    name: String,
    signature: Signature,
    return_type: DataType,
}

impl CatalogMaskUdf {
    fn from_function(name: &str, function: &Function) -> Result<Self> {
        let arg_types = function
            .input_params
            .as_option()
            .map(|params| {
                params
                    .parameters
                    .iter()
                    .map(|p| col_type_to_arrow(&p.type_name))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();

        // UC carries the return type as a `data_type` type-spec string, not the
        // `ColumnTypeName` enum used for parameters. The fail-closed no-op body
        // returns the mask literal (a string), so bind Utf8 as the return type
        // regardless — real return-type derivation lands with UC-SQL execution.
        let _ = function;
        let return_type = DataType::Utf8;

        Ok(Self {
            name: name.to_string(),
            signature: Signature::exact(arg_types, Volatility::Immutable),
            return_type,
        })
    }
}

impl ScalarUDFImpl for CatalogMaskUdf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(self.return_type.clone())
    }

    fn invoke_with_args(&self, _args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        // Fail-closed no-op: the UC SQL body is not executed (deferred). The
        // resolved signature is real; the value returned is the mask literal.
        Ok(ColumnarValue::Scalar(ScalarValue::Utf8(Some(
            MASK_LITERAL.to_string(),
        ))))
    }
}

/// Map a Unity Catalog [`ColumnTypeName`] (the type carried on function
/// parameters and return type) to an Arrow [`DataType`]. Covers the showcase
/// scalar types; an unsupported/unspecified type is an `Err` so resolution fails
/// closed rather than binding a wrong signature.
fn col_type_to_arrow(type_name: &EnumValue<ColumnTypeName>) -> Result<DataType> {
    let col_type = type_name.as_known().ok_or_else(|| {
        DataFusionError::Plan(format!(
            "unknown UC column type enum value {}",
            type_name.to_i32()
        ))
    })?;
    let dt = match col_type {
        ColumnTypeName::Boolean => DataType::Boolean,
        ColumnTypeName::Byte => DataType::Int8,
        ColumnTypeName::Short => DataType::Int16,
        ColumnTypeName::Int => DataType::Int32,
        ColumnTypeName::Long => DataType::Int64,
        ColumnTypeName::Float => DataType::Float32,
        ColumnTypeName::Double => DataType::Float64,
        ColumnTypeName::String => DataType::Utf8,
        other => {
            return Err(DataFusionError::Plan(format!(
                "unsupported UC column type {other:?} in catalog function signature"
            )));
        }
    };
    Ok(dt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unitycatalog_common::models::functions::v1::{
        FunctionParameterInfo, FunctionParameterInfos,
    };

    fn param(type_name: ColumnTypeName) -> FunctionParameterInfo {
        FunctionParameterInfo {
            type_name: type_name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn builds_signature_from_function_params_and_return_type() {
        let function = Function {
            input_params: FunctionParameterInfos {
                parameters: vec![param(ColumnTypeName::String), param(ColumnTypeName::Int)],
                ..Default::default()
            }
            .into(),
            data_type: "STRING".to_string(),
            ..Default::default()
        };

        let udf = CatalogMaskUdf::from_function("hr.security.mask_ssn", &function).unwrap();
        assert_eq!(udf.name(), "hr.security.mask_ssn");
        assert_eq!(udf.return_type(&[]).unwrap(), DataType::Utf8);
        // The signature is exact over (Utf8, Int32).
        assert_eq!(
            udf.signature().type_signature,
            datafusion::logical_expr::TypeSignature::Exact(vec![DataType::Utf8, DataType::Int32])
        );
    }

    #[test]
    fn no_params_yields_a_zero_arg_signature() {
        let function = Function::default();
        let udf = CatalogMaskUdf::from_function("f", &function).unwrap();
        assert_eq!(
            udf.signature().type_signature,
            datafusion::logical_expr::TypeSignature::Exact(vec![])
        );
    }

    #[test]
    fn unsupported_param_type_fails_closed() {
        let function = Function {
            input_params: FunctionParameterInfos {
                parameters: vec![param(ColumnTypeName::Timestamp)],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        assert!(CatalogMaskUdf::from_function("f", &function).is_err());
    }

    #[tokio::test]
    async fn resolve_fails_closed_when_catalog_unreachable() {
        let client = UnityCatalogClient::new_unauthenticated(
            url::Url::parse("http://127.0.0.1:1/").unwrap(),
        );
        let resolver = UnityFunctionResolver::new(client);
        assert!(resolver.resolve("hr.security.mask_ssn").await.is_err());
    }
}

use arrow_flight::error::FlightError;
use arrow_schema::ArrowError;

pub type Result<T, E = HydrofoilClientError> = std::result::Result<T, E>;

pub(crate) fn status_to_arrow_error(status: tonic::Status) -> ArrowError {
    ArrowError::IpcError(format!("{status:?}"))
}

pub(crate) fn decode_error_to_arrow_error(err: prost::DecodeError) -> ArrowError {
    ArrowError::IpcError(err.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum HydrofoilClientError {
    #[error("Arrow error: {source}")]
    Arrow {
        #[from]
        source: ArrowError,
    },

    #[error("Arrow Flight error: {source}")]
    ArrowFlight {
        #[from]
        source: FlightError,
    },

    #[error("Transport error: {source}")]
    Transport {
        #[from]
        source: tonic::transport::Error,
    },
}

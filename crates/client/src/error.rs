pub type Result<T, E = HydrofoilClientError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum HydrofoilClientError {
    #[error("Arrow error: {source}")]
    Arrow {
        #[from]
        source: arrow::error::ArrowError,
    },

    #[error("Arrow Flight error: {source}")]
    ArrowFlight {
        #[from]
        source: arrow_flight::error::FlightError,
    },

    #[error("Transport error: {source}")]
    Transport {
        #[from]
        source: tonic::transport::Error,
    },
}

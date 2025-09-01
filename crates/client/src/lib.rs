use arrow_flight::flight_service_client::FlightServiceClient;
use arrow_flight::sql::client::FlightSqlServiceClient;
use tonic::transport::{Channel, Endpoint};

use crate::error::Result;

mod error;

#[derive(Debug, Clone)]
pub struct Client {
    client: FlightSqlServiceClient<Channel>,
}

impl Client {
    pub async fn try_new<D>(endpoint: D) -> Result<Self>
    where
        D: TryInto<Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        let inner = FlightServiceClient::connect(endpoint).await?;
        let client = FlightSqlServiceClient::new_from_inner(inner);
        Ok(Self { client })
    }

    pub async fn handshake(&mut self) -> Result<()> {
        let result = self.client.handshake("user", "password").await?;
        println!("Handshake result: {:?}", result);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_works() {
        let mut client = Client::try_new("http://localhost:50051").await.unwrap();
        client.handshake().await.unwrap();
    }
}

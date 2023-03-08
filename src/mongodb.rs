use mongodb::{options::ClientOptions, Client, Database};

use std::error::Error;

pub async fn connect_to_database() -> Result<Database, Box<dyn Error>> {
    let connection_url = std::env::var("MONGODB_URI").unwrap();
    let database_name = std::env::var("MONGODB_DB").unwrap();

    let mut client_options = ClientOptions::parse(connection_url).await?;

    client_options.app_name = Some("Blobscan".to_string());

    let client = Client::with_options(client_options)?;

    Ok(client.database(&database_name))
}

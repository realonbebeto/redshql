use std::sync::Arc;

use pgwire::tokio::process_socket;
use redshqlx::{
    handler::{RedshqlHandler, RedshqlHandlerFactory},
    init_tracing_subscriber,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing_subscriber();

    let pg_dsn = std::env::var("PG_DSN")?;

    let (pg_client, pg_conn) = tokio_postgres::connect(&pg_dsn, tokio_postgres::NoTls).await?;
    let redshql = RedshqlHandler::new(pg_client);

    let factory = Arc::new(RedshqlHandlerFactory {
        handler: Arc::new(redshql),
    });

    let server_addr = std::env::var("REDSHQL_ADDRESS").unwrap_or(String::from("0.0.0.0:5439"));

    let listener = TcpListener::bind(&server_addr).await.unwrap();
    tracing::info!("Listening to {}", &server_addr);

    tokio::spawn(async move {
        if let Err(e) = pg_conn.await {
            tracing::error!("postgres connection error: {e}");
        }
    });

    loop {
        let incoming_socket = listener.accept().await?;
        let factory_ref = factory.clone();

        tokio::spawn(async move { process_socket(incoming_socket.0, None, factory_ref).await });
    }
}

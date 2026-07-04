use std::sync::Arc;

use async_trait::async_trait;
use pgwire::{
    api::{
        ClientInfo, ClientPortalStore, PgWireServerHandlers,
        query::SimpleQueryHandler,
        results::{Response, Tag},
        store::PortalStore,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};
use sqlparser::{ast::Statement, dialect::RedshiftSqlDialect, parser::Parser};
use tokio_postgres::Client as PgClient;

use crate::{
    load::{execute_create_table, execute_s3_copy, execute_select},
    statements::{copy::try_parse_redshift_copy, create::try_parse_redshift_create_table},
};

pub struct RedshqlHandlerFactory {
    pub handler: Arc<RedshqlHandler>,
}

impl PgWireServerHandlers for RedshqlHandlerFactory {
    fn simple_query_handler(&self) -> Arc<impl pgwire::api::query::SimpleQueryHandler> {
        self.handler.clone()
    }
}

pub struct RedshqlHandler {
    pg: PgClient,
}

impl RedshqlHandler {
    pub fn new(pg: PgClient) -> Self {
        Self { pg }
    }
}

#[async_trait]
impl SimpleQueryHandler for RedshqlHandler {
    async fn do_query<C>(&self, _client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
    {
        let dialect = RedshiftSqlDialect {};

        let start = Parser::parse_sql(&dialect, query).map_err(|e| {
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".into(),
                "26000".into(),
                e.to_string(),
            )))
        })?;

        match start.first() {
            Some(Statement::Query(_)) => {
                let result = execute_select(query, &self.pg).await?;

                Ok(vec![result])
            }
            Some(Statement::CreateTable(_)) => {
                tracing::info!("{}", query);
                let parser = Parser::new(&dialect).try_with_sql(query).map_err(|e| {
                    PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".into(),
                        "26000".into(),
                        e.to_string(),
                    )))
                })?;

                let create = try_parse_redshift_create_table(parser).map_err(|e| {
                    PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".into(),
                        "42601".into(),
                        format!("Unsupported: {:?}", e),
                    )))
                })?;

                tracing::info!("Parsing create SQL");

                let res = execute_create_table(create, &self.pg).await?;

                return Ok(vec![res]);
            }
            Some(Statement::Copy { .. }) => {
                let parser = Parser::new(&dialect).try_with_sql(query).map_err(|e| {
                    PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".into(),
                        "26000".into(),
                        e.to_string(),
                    )))
                })?;

                let r_copy = try_parse_redshift_copy(parser).map_err(|e| {
                    PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".into(),
                        "42601".into(),
                        format!("Unsupported: {:?}", e),
                    )))
                })?;

                tracing::info!("Parsing copy SQL");

                let n = execute_s3_copy(&r_copy, &self.pg).await.map_err(|e| {
                    PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".into(),
                        "42601".into(),
                        format!("Unsupported: {:?}", e),
                    )))
                })?;

                return Ok(vec![Response::Execution(Tag::new("copy").with_rows(n))]);
            }

            Some(other) => Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                "WARNING".into(),
                "01000".into(),
                format!("Unsupported: {:?}", other),
            ))))?,
            None => Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".into(),
                "42601".into(),
                "Invalid query".into(),
            ))))?,
        }
    }
}

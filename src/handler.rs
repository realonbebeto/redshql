use std::sync::Arc;

use async_trait::async_trait;
use pgwire::{
    api::{
        ClientInfo, ClientPortalStore, PgWireServerHandlers, Type,
        portal::Portal,
        query::{ExtendedQueryHandler, SimpleQueryHandler},
        results::{DescribePortalResponse, DescribeStatementResponse, FieldInfo, Response, Tag},
        stmt::{NoopQueryParser, StoredStatement},
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

    fn extended_query_handler(&self) -> Arc<impl pgwire::api::query::ExtendedQueryHandler> {
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
impl ExtendedQueryHandler for RedshqlHandler {
    type Statement = String;
    type QueryParser = NoopQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        Arc::new(NoopQueryParser::new())
    }

    async fn do_query<C>(
        &self,
        _client: &mut C,
        portal: &Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let query = &portal.statement.statement;

        if query.trim().is_empty() {
            return Ok(Response::EmptyQuery);
        }

        tracing::info!("{}", query);

        execute_statement(query, &self.pg).await
    }

    async fn do_describe_statement<C>(
        &self,
        _client: &mut C,
        stmt: &StoredStatement<Self::Statement>,
    ) -> PgWireResult<DescribeStatementResponse>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let dialect = RedshiftSqlDialect {};

        let start = Parser::parse_sql(&dialect, &stmt.statement).map_err(|e| {
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".into(),
                "26000".into(),
                e.to_string(),
            )))
        })?;

        if let Some(Statement::Copy { .. }) = start.first() {
            return Ok(DescribeStatementResponse::new(vec![], vec![]));
        }

        let prepared = self
            .pg
            .prepare(&stmt.statement)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;

        let param_types: Vec<Type> = prepared.params().to_vec();

        let fields: Vec<FieldInfo> = prepared
            .columns()
            .iter()
            .map(|col| {
                FieldInfo::new(
                    col.name().to_owned(),
                    None,
                    None,
                    col.type_().clone(),
                    pgwire::api::results::FieldFormat::Text,
                )
            })
            .collect();

        Ok(DescribeStatementResponse::new(param_types, fields))
    }

    async fn do_describe_portal<C>(
        &self,
        _client: &mut C,
        portal: &Portal<Self::Statement>,
    ) -> PgWireResult<DescribePortalResponse>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let dialect = RedshiftSqlDialect {};

        let start = Parser::parse_sql(&dialect, &portal.statement.statement).map_err(|e| {
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".into(),
                "26000".into(),
                e.to_string(),
            )))
        })?;

        if let Some(Statement::Copy { .. }) = start.first() {
            return Ok(DescribePortalResponse::new(vec![]));
        }

        let prepared = self
            .pg
            .prepare(&portal.statement.statement)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;

        let fields: Vec<FieldInfo> = prepared
            .columns()
            .iter()
            .enumerate()
            .map(|(idx, col)| {
                FieldInfo::new(
                    col.name().to_owned(),
                    None,
                    None,
                    col.type_().clone(),
                    portal.result_column_format.format_for(idx),
                )
            })
            .collect();

        Ok(DescribePortalResponse::new(fields))
    }
}

#[async_trait]
impl SimpleQueryHandler for RedshqlHandler {
    async fn do_query<C>(&self, _client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
    {
        tracing::info!("HH");
        if query.trim().is_empty() {
            return Ok(vec![Response::EmptyQuery]);
        }

        tracing::info!("{}", query);

        Ok(vec![execute_statement(query, &self.pg).await?])
    }
}

async fn execute_statement(query: &str, pg: &PgClient) -> PgWireResult<Response> {
    let dialect = RedshiftSqlDialect {};

    let start = Parser::parse_sql(&dialect, query).map_err(|e| {
        PgWireError::UserError(Box::new(ErrorInfo::new(
            "ERROR".into(),
            "26000".into(),
            e.to_string(),
        )))
    })?;

    match start.first() {
        Some(Statement::Query(_)) => execute_select(query, pg).await,
        Some(Statement::CreateTable(_)) => {
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
            execute_create_table(create, pg).await
        }
        Some(Statement::Copy { .. }) => {
            tracing::info!("Try parsing");
            let parser = Parser::new(&dialect).try_with_sql(query).map_err(|e| {
                PgWireError::UserError(Box::new(ErrorInfo::new(
                    "ERROR".into(),
                    "26000".into(),
                    e.to_string(),
                )))
            })?;
            tracing::info!("Parsing");
            let r_copy = try_parse_redshift_copy(parser).map_err(|e| {
                PgWireError::UserError(Box::new(ErrorInfo::new(
                    "ERROR".into(),
                    "42601".into(),
                    format!("Unsupported: {:?}", e),
                )))
            })?;
            tracing::info!("Copy parsed");
            let n = execute_s3_copy(&r_copy, pg).await.map_err(|e| {
                PgWireError::UserError(Box::new(ErrorInfo::new(
                    "ERROR".into(),
                    "42601".into(),
                    format!("Unsupported: {:?}", e),
                )))
            })?;
            Ok(Response::Execution(Tag::new("copy").with_rows(n)))
        }
        Some(other) => Err(PgWireError::UserError(Box::new(ErrorInfo::new(
            "WARNING".into(),
            "01000".into(),
            format!("Unsupported: {:?}", other),
        )))),
        None => Err(PgWireError::UserError(Box::new(ErrorInfo::new(
            "ERROR".into(),
            "42601".into(),
            "Invalid query".into(),
        )))),
    }
}

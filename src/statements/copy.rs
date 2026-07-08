use sqlparser::{
    ast::ObjectName,
    keywords::Keyword,
    parser::{Parser, ParserError},
    tokenizer::Token,
};

use crate::statements::{
    RedshqlParseError, expect_word, parse_number_literal, parse_string_literal,
};

#[derive(Debug, Clone)]
pub struct RedshqlCopy {
    pub table: ObjectName,
    pub s3_uri: String,
    pub credentials: Option<String>,
    pub format: CopyFormat,
    pub max_error: u32,
    pub region: Option<String>,
    pub terminator: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CopyFormat {
    Csv,
    Json,
    Orc,
    Parquet,
    Other(String),
}

impl CopyFormat {
    pub fn extension(&self) -> &str {
        match self {
            Self::Csv => ".csv",
            Self::Json => ".json",
            Self::Orc => ".orc",
            Self::Parquet => ".parquet",
            Self::Other(v) => v,
        }
    }
}

impl From<ParserError> for RedshqlParseError {
    fn from(value: ParserError) -> Self {
        Self::Sql(value)
    }
}

pub fn try_parse_redshift_copy(mut parser: Parser) -> Result<RedshqlCopy, RedshqlParseError> {
    if !parser.parse_keyword(Keyword::COPY) {
        Err(RedshqlParseError::NotACopyStatement)?
    }

    let table = parser.parse_object_name(false)?;

    parser.expect_keyword(Keyword::FROM)?;

    let s3_uri = parse_string_literal(&mut parser)?;

    if !s3_uri.starts_with("s3://") {
        Err(RedshqlParseError::Malformed(format!(
            "expected s3:// URI, got {s3_uri}"
        )))?
    }

    let mut credentials = None;
    // Redshift's real default when FORMAT is omitted
    let mut format = CopyFormat::Csv;
    let mut max_error = 0u32;
    let mut region = None;
    let mut terminator = false;

    // Real Redshift COPY options are unordered after FROM — loop until EOF
    while parser.peek_token() != Token::EOF {
        let word = expect_word(&mut parser)?;

        tracing::info!("{}", word);

        match word.to_uppercase().as_str() {
            "CREDENTIALS" | "IAM_ROLE" => {
                credentials = Some(parse_string_literal(&mut parser)?);
            }
            "FORMAT" => {
                let _ = parser.parse_keyword(Keyword::AS); // "FORMAT AS X" or bare "FORMAT X"
                format = match expect_word(&mut parser)?.to_uppercase().as_str() {
                    "PARQUET" => CopyFormat::Parquet,
                    "CSV" => CopyFormat::Csv,
                    "JSON" => CopyFormat::Json,
                    other => CopyFormat::Other(other.to_string()),
                };
            }
            "MAXERROR" => max_error = parse_number_literal(&mut parser)?,
            "REGION" => region = Some(parse_string_literal(&mut parser)?),
            // clauses you don't emit but want to tolerate rather than error on
            "GZIP" | "COMPUPDATE" | "STATUPDATE" => {
                let _ = parser.parse_one_of_keywords(&[Keyword::ON, Keyword::OFF]);
            }
            "IGNOREHEADER" => {
                parse_number_literal(&mut parser)?;
            }
            "DELIMITER" => {
                parse_string_literal(&mut parser)?;
            }
            "SEMICOLON" => terminator = true,
            unknown => {
                return Err(RedshqlParseError::Malformed(format!(
                    "unsupported COPY clause: {unknown}"
                )));
            }
        }
    }

    Ok(RedshqlCopy {
        table,
        s3_uri,
        credentials,
        format,
        max_error,
        region,
        terminator,
    })
}

#[cfg(test)]
mod tests {
    use sqlparser::dialect::RedshiftSqlDialect;

    use super::*;

    // ---------- COPY tests ----------

    #[test]
    fn copy_minimal() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY my_table FROM 's3://bucket/path/' IAM_ROLE 'arn:aws:iam::123:role/x'";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let copy = try_parse_redshift_copy(parser).unwrap();
        assert_eq!(copy.table.to_string(), "my_table");
        assert_eq!(copy.s3_uri, "s3://bucket/path/");
        assert_eq!(copy.credentials.as_deref(), Some("arn:aws:iam::123:role/x"));
        assert_eq!(copy.format, CopyFormat::Csv); // default
        assert_eq!(copy.max_error, 0);
        assert_eq!(copy.region, None);
    }

    #[test]
    fn copy_with_all_options_any_order() {
        let dialect = RedshiftSqlDialect {};
        let sql = "COPY schema.my_table FROM 's3://bucket/data/' \
                   FORMAT AS PARQUET \
                   MAXERROR 10 \
                   REGION 'us-east-1' \
                   CREDENTIALS 'aws_iam_role=arn:aws:iam::123:role/x'";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let copy = try_parse_redshift_copy(parser).unwrap();

        assert_eq!(copy.table.to_string(), "schema.my_table");
        assert_eq!(copy.format, CopyFormat::Parquet);
        assert_eq!(copy.max_error, 10);
        assert_eq!(copy.region.as_deref(), Some("us-east-1"));
        assert!(copy.credentials.is_some());
    }

    #[test]
    fn copy_options_are_truly_unordered() {
        let dialect = RedshiftSqlDialect {};

        // same options as above, reordered — should parse identically
        let sql = "COPY schema.my_table FROM 's3://bucket/data/' \
                   MAXERROR 10 \
                   CREDENTIALS 'aws_iam_role=arn:aws:iam::123:role/x' \
                   REGION 'us-east-1' \
                   FORMAT AS PARQUET";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let copy = try_parse_redshift_copy(parser).unwrap();

        assert_eq!(copy.format, CopyFormat::Parquet);
        assert_eq!(copy.max_error, 10);
        assert_eq!(copy.region.as_deref(), Some("us-east-1"));
    }

    #[test]
    fn copy_format_bare_without_as() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY t FROM 's3://b/p' FORMAT JSON";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let copy = try_parse_redshift_copy(parser).unwrap();

        assert_eq!(copy.format, CopyFormat::Json);
    }

    #[test]
    fn copy_format_unknown_falls_back_to_other() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY t FROM 's3://b/p' FORMAT ORC";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let copy = try_parse_redshift_copy(parser).unwrap();

        assert_eq!(copy.format, CopyFormat::Other("ORC".to_string()));
    }

    #[test]
    fn copy_tolerated_clauses_do_not_error() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY t FROM 's3://b/p' \
                   IGNOREHEADER 1 \
                   DELIMITER '|' \
                   GZIP \
                   COMPUPDATE OFF \
                   STATUPDATE ON";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let copy = try_parse_redshift_copy(parser).unwrap();
        assert_eq!(copy.table.to_string(), "t");
    }

    #[test]
    fn copy_rejects_non_copy_statement() {
        let dialect = RedshiftSqlDialect {};

        let sql = "SELECT * FROM t";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_copy(parser).unwrap_err();
        assert!(matches!(err, RedshqlParseError::NotACopyStatement));
    }

    #[test]
    fn copy_rejects_non_s3_uri() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY t FROM 'https://example.com/data.csv'";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_copy(parser).unwrap_err();
        assert!(matches!(err, RedshqlParseError::Malformed(_)));
    }

    #[test]
    fn copy_rejects_unsupported_clause() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY t FROM 's3://b/p' TOTALLY_MADE_UP_OPTION 5";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_copy(parser).unwrap_err();
        assert!(matches!(err, RedshqlParseError::Malformed(_)));
    }

    #[test]
    fn copy_rejects_missing_from() {
        let dialect = RedshiftSqlDialect {};

        let sql = "COPY t 's3://b/p'";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        assert!(try_parse_redshift_copy(parser).is_err());
    }
}

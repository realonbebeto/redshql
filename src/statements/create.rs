use sqlparser::{
    ast::{DataType, Expr, Ident, ObjectName},
    keywords::Keyword,
    parser::Parser,
    tokenizer::Token,
};

use crate::statements::{RedshqlParseError, expect_word, parse_number_literal};

#[derive(Debug, Clone)]
pub struct RedshqlCreateTable {
    pub table: ObjectName,
    pub if_not_exists: bool,
    pub columns: Vec<RedshiftColumnDef>,
    pub table_kind: TableKind, // TEMP / LOCAL TEMP / regular
    pub dist_style: Option<DistStyle>,
    pub dist_key: Option<Ident>, // column name, only valid when dist_style == Key
    pub sort_key: Option<SortKey>,
    pub backup: Option<bool>, // BACKUP YES | NO
}

#[derive(Debug, Clone)]
pub struct RedshiftColumnDef {
    pub name: Ident,
    pub data_type: DataType,
    pub encoding: Option<ColumnEncoding>, // ENCODE ZSTD, LZO, RAW, etc.
    pub not_null: bool,
    pub default: Option<Expr>,
    pub identity: Option<IdentitySpec>, // IDENTITY(seed, step)
    pub primary_key: bool,
    pub references: Option<ObjectName>, // simplified FK target
}

#[derive(Debug, Clone)]
pub enum TableKind {
    Regular,
    Temp,
    LocalTemp,
}

#[derive(Debug, Clone)]
pub enum DistStyle {
    Even,
    Key,
    All,
    Auto,
}

#[derive(Debug, Clone)]
pub enum SortKey {
    Compound(Vec<Ident>),
    Interleaved(Vec<Ident>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnEncoding {
    Raw,
    Bytedict,
    Delta,
    Delta32k,
    Lzo,
    Mostly8,
    Mostly16,
    Mostly32,
    Runlength,
    Text255,
    Text32k,
    Zstd,
    Az64,
}

#[derive(Debug, Clone)]
pub struct IdentitySpec {
    pub seed: i64,
    pub step: i64,
}

// Parse CREATE
pub fn try_parse_redshift_create_table(
    mut parser: Parser,
) -> Result<RedshqlCreateTable, RedshqlParseError> {
    if !parser.parse_keyword(Keyword::CREATE) {
        Err(RedshqlParseError::NotACreateStatement)?
    }

    let table_kind =
        if parser.parse_keyword(Keyword::TEMPORARY) || parser.parse_keyword(Keyword::TEMP) {
            TableKind::Temp
        } else if parser.parse_keywords(&[Keyword::LOCAL, Keyword::TEMPORARY])
            || parser.parse_keywords(&[Keyword::LOCAL, Keyword::TEMP])
        {
            TableKind::LocalTemp
        } else {
            TableKind::Regular
        };

    parser.expect_keyword(Keyword::TABLE)?;

    let if_not_exists = parser.parse_keywords(&[Keyword::IF, Keyword::NOT, Keyword::EXISTS]);

    let table = parser.parse_object_name(false)?;

    let columns = parse_column_list(&mut parser)?;

    let mut dist_style = None;
    let mut dist_key = None;
    let mut sort_key = None;
    let mut backup = None;

    // Table-level clauses after the column list are unordered, same as COPY options
    while parser.peek_token() != Token::EOF {
        let word = expect_word(&mut parser)?;
        match word.to_uppercase().as_str() {
            "DISTSTYLE" => {
                dist_style = Some(match expect_word(&mut parser)?.to_uppercase().as_str() {
                    "EVEN" => DistStyle::Even,
                    "KEY" => DistStyle::Key,
                    "ALL" => DistStyle::All,
                    "AUTO" => DistStyle::Auto,
                    other => Err(RedshqlParseError::Malformed(format!(
                        "unknown DISTSTYLE: {other}"
                    )))?,
                });
            }
            "DISTKEY" => {
                parser.expect_token(&Token::LParen)?;
                dist_key = Some(parser.parse_identifier()?);
                parser.expect_token(&Token::RParen)?;
            }
            "SORTKEY" => {
                sort_key = Some(SortKey::Compound(parse_ident_list(&mut parser)?));
            }
            "COMPOUND" => {
                parser.expect_keyword(Keyword::SORTKEY)?;
                sort_key = Some(SortKey::Compound(parse_ident_list(&mut parser)?));
            }
            "INTERLEAVED" => {
                parser.expect_keyword(Keyword::SORTKEY)?;
                sort_key = Some(SortKey::Interleaved(parse_ident_list(&mut parser)?));
            }
            "BACKUP" => {
                backup = Some(match expect_word(&mut parser)?.to_uppercase().as_str() {
                    "YES" => true,
                    "NO" => false,
                    other => Err(RedshqlParseError::Malformed(format!(
                        "expected YES|NO after BACKUP, got {other}"
                    )))?,
                });
            }
            "ENCODE" => {
                // table-level ENCODE AUTO|NONE — tolerate, not modeled per-table yet
                let _ = expect_word(&mut parser)?;
            }
            unknown => {
                return Err(RedshqlParseError::Malformed(format!(
                    "unsupported CREATE TABLE clause: {unknown}"
                )));
            }
        }
    }

    Ok(RedshqlCreateTable {
        table,
        if_not_exists,
        columns,
        table_kind,
        dist_style,
        dist_key,
        sort_key,
        backup,
    })
}

fn parse_ident_list(parser: &mut Parser) -> Result<Vec<Ident>, RedshqlParseError> {
    parser.expect_token(&Token::LParen)?;
    let idents = parser.parse_comma_separated(Parser::parse_identifier)?;
    parser.expect_token(&Token::RParen)?;
    Ok(idents)
}

fn parse_column_list(parser: &mut Parser) -> Result<Vec<RedshiftColumnDef>, RedshqlParseError> {
    parser.expect_token(&Token::LParen)?;
    let mut columns = Vec::new();

    loop {
        let name = parser.parse_identifier()?;
        let data_type = parser.parse_data_type()?;

        let mut encoding = None;
        let mut not_null = false;
        let mut default = None;
        let mut identity = None;
        let mut primary_key = false;
        let mut references = None;

        // Column constraints are unordered too — loop until comma or close paren
        loop {
            match parser.peek_token().token {
                Token::Comma | Token::RParen => break,
                _ => {}
            }
            let word = expect_word(parser)?;
            match word.to_uppercase().as_str() {
                "NOT" => {
                    parser.expect_keyword(Keyword::NULL)?;
                    not_null = true;
                }
                "NULL" => not_null = false,
                "DEFAULT" => default = Some(parser.parse_expr()?),
                "IDENTITY" => {
                    parser.expect_token(&Token::LParen)?;
                    let seed = parse_number_literal(parser)? as i64;
                    parser.expect_token(&Token::Comma)?;
                    let step = parse_number_literal(parser)? as i64;
                    parser.expect_token(&Token::RParen)?;
                    identity = Some(IdentitySpec { seed, step });
                }
                "ENCODE" => {
                    encoding = Some(match expect_word(parser)?.to_uppercase().as_str() {
                        "RAW" => ColumnEncoding::Raw,
                        "BYTEDICT" => ColumnEncoding::Bytedict,
                        "DELTA" => ColumnEncoding::Delta,
                        "DELTA32K" => ColumnEncoding::Delta32k,
                        "LZO" => ColumnEncoding::Lzo,
                        "MOSTLY8" => ColumnEncoding::Mostly8,
                        "MOSTLY16" => ColumnEncoding::Mostly16,
                        "MOSTLY32" => ColumnEncoding::Mostly32,
                        "RUNLENGTH" => ColumnEncoding::Runlength,
                        "TEXT255" => ColumnEncoding::Text255,
                        "TEXT32K" => ColumnEncoding::Text32k,
                        "ZSTD" => ColumnEncoding::Zstd,
                        "AZ64" => ColumnEncoding::Az64,
                        other => Err(RedshqlParseError::Malformed(format!(
                            "unknown ENCODE: {other}"
                        )))?,
                    });
                }
                "PRIMARY" => {
                    parser.expect_keyword(Keyword::KEY)?;
                    primary_key = true;
                }
                "REFERENCES" => {
                    references = Some(parser.parse_object_name(false)?);
                }
                // column-level DISTKEY/SORTKEY flags — tolerate, table-level fields win
                "DISTKEY" | "SORTKEY" => {}
                unknown => {
                    return Err(RedshqlParseError::Malformed(format!(
                        "unsupported column constraint: {unknown}"
                    )));
                }
            }
        }

        columns.push(RedshiftColumnDef {
            name,
            data_type,
            encoding,
            not_null,
            default,
            identity,
            primary_key,
            references,
        });

        if parser.consume_token(&Token::Comma) {
            continue;
        }
        parser.expect_token(&Token::RParen)?;
        break;
    }

    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::dialect::RedshiftSqlDialect;

    // ---------- CREATE TABLE tests ----------

    #[test]
    fn create_minimal_single_column() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE my_table (id INTEGER)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert_eq!(create.table.to_string(), "my_table");
        assert!(matches!(create.table_kind, TableKind::Regular));
        assert!(!create.if_not_exists);
        assert_eq!(create.columns.len(), 1);
        assert_eq!(create.columns[0].name.to_string(), "id");
        assert!(create.dist_style.is_none());
    }

    #[test]
    fn create_if_not_exists() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE IF NOT EXISTS my_table (id INTEGER)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();

        let create = try_parse_redshift_create_table(parser).unwrap();
        assert!(create.if_not_exists);
    }

    #[test]
    fn create_temp_table() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TEMP TABLE staging (id INTEGER)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert!(matches!(create.table_kind, TableKind::Temp));
    }

    #[test]
    fn create_local_temp_table() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE LOCAL TEMPORARY TABLE staging (id INTEGER)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert!(matches!(create.table_kind, TableKind::LocalTemp));
    }

    #[test]
    fn create_multiple_columns_with_constraints() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE orders (
            id BIGINT IDENTITY(1, 1) PRIMARY KEY,
            customer_id BIGINT NOT NULL REFERENCES customers,
            amount NUMERIC(18, 2) DEFAULT 0 ENCODE ZSTD,
            note VARCHAR(256)
        )";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert_eq!(create.columns.len(), 4);

        let id = &create.columns[0];
        assert!(id.primary_key);
        assert!(matches!(
            id.identity,
            Some(IdentitySpec { seed: 1, step: 1 })
        ));

        let customer_id = &create.columns[1];
        assert!(customer_id.not_null);
        assert_eq!(
            customer_id.references.as_ref().map(|r| r.to_string()),
            Some("customers".to_string())
        );

        let amount = &create.columns[2];
        assert!(amount.default.is_some());
        assert!(matches!(amount.encoding, Some(ColumnEncoding::Zstd)));

        let note = &create.columns[3];
        assert!(!note.not_null);
        assert!(note.default.is_none());
    }

    #[test]
    fn create_diststyle_and_distkey() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (id INT, region VARCHAR(10)) \
                   DISTSTYLE KEY DISTKEY(region)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();
        assert!(matches!(create.dist_style, Some(DistStyle::Key)));
        assert_eq!(
            create.dist_key.as_ref().map(|i| i.to_string()),
            Some("region".to_string())
        );
    }

    #[test]
    fn create_diststyle_all() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE lookup (id INT) DISTSTYLE ALL";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert!(matches!(create.dist_style, Some(DistStyle::All)));
    }

    #[test]
    fn create_compound_sortkey() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (a INT, b INT) SORTKEY(a, b)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();
        match create.sort_key {
            Some(SortKey::Compound(cols)) => {
                assert_eq!(
                    cols.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
                    vec!["a", "b"]
                );
            }
            other => panic!("expected Compound sort key, got {other:?}"),
        }
    }

    #[test]
    fn create_explicit_compound_sortkey() {
        let dialect = RedshiftSqlDialect {};
        let sql = "CREATE TABLE t (a INT, b INT) COMPOUND SORTKEY(a, b)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert!(matches!(create.sort_key, Some(SortKey::Compound(_))));
    }

    #[test]
    fn create_interleaved_sortkey() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (a INT, b INT) INTERLEAVED SORTKEY(a, b)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        match create.sort_key {
            Some(SortKey::Interleaved(cols)) => assert_eq!(cols.len(), 2),
            other => panic!("expected Interleaved sort key, got {other:?}"),
        }
    }

    #[test]
    fn create_backup_yes_no() {
        let dialect = RedshiftSqlDialect {};

        let sql_yes = "CREATE TABLE t (id INT) BACKUP YES";
        let parser = Parser::new(&dialect).try_with_sql(sql_yes).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert_eq!(create.backup, Some(true));

        let sql_no = "CREATE TABLE t (id INT) BACKUP NO";
        let parser = Parser::new(&dialect).try_with_sql(sql_no).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert_eq!(create.backup, Some(false));
    }

    #[test]
    fn create_table_clauses_unordered() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (id INT, region VARCHAR(10)) \
                   BACKUP NO \
                   SORTKEY(id) \
                   DISTSTYLE KEY \
                   DISTKEY(region)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let create = try_parse_redshift_create_table(parser).unwrap();

        assert_eq!(create.backup, Some(false));
        assert!(create.sort_key.is_some());
        assert!(matches!(create.dist_style, Some(DistStyle::Key)));
        assert!(create.dist_key.is_some());
    }

    #[test]
    fn create_all_encode_variants_parse() {
        let dialect = RedshiftSqlDialect {};

        let encodings = [
            ("RAW", ColumnEncoding::Raw),
            ("BYTEDICT", ColumnEncoding::Bytedict),
            ("DELTA", ColumnEncoding::Delta),
            ("DELTA32K", ColumnEncoding::Delta32k),
            ("LZO", ColumnEncoding::Lzo),
            ("MOSTLY8", ColumnEncoding::Mostly8),
            ("MOSTLY16", ColumnEncoding::Mostly16),
            ("MOSTLY32", ColumnEncoding::Mostly32),
            ("RUNLENGTH", ColumnEncoding::Runlength),
            ("TEXT255", ColumnEncoding::Text255),
            ("TEXT32K", ColumnEncoding::Text32k),
            ("ZSTD", ColumnEncoding::Zstd),
            ("AZ64", ColumnEncoding::Az64),
        ];
        for (kw, expected) in encodings {
            let sql = format!("CREATE TABLE t (id INT ENCODE {kw})");
            let parser = Parser::new(&dialect).try_with_sql(&sql).unwrap();
            let create = try_parse_redshift_create_table(parser).unwrap();

            assert_eq!(
                create.columns[0].encoding,
                Some(expected),
                "failed for ENCODE {kw}"
            );
        }
    }

    #[test]
    fn create_rejects_non_create_statement() {
        let dialect = RedshiftSqlDialect {};

        let sql = "SELECT * FROM t";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_create_table(parser).unwrap_err();

        assert!(matches!(err, RedshqlParseError::NotACreateStatement));
    }

    #[test]
    fn create_rejects_missing_table_keyword() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE my_table (id INT)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();

        assert!(try_parse_redshift_create_table(parser).is_err());
    }

    #[test]
    fn create_rejects_unknown_diststyle() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (id INT) DISTSTYLE BOGUS";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_create_table(parser).unwrap_err();

        assert!(matches!(err, RedshqlParseError::Malformed(_)));
    }

    #[test]
    fn create_rejects_unsupported_table_clause() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (id INT) TOTALLY_MADE_UP_CLAUSE";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_create_table(parser).unwrap_err();

        assert!(matches!(err, RedshqlParseError::Malformed(_)));
    }

    #[test]
    fn create_rejects_unsupported_column_constraint() {
        let dialect = RedshiftSqlDialect {};

        let sql = "CREATE TABLE t (id INT TOTALLY_MADE_UP_CONSTRAINT)";
        let parser = Parser::new(&dialect).try_with_sql(sql).unwrap();
        let err = try_parse_redshift_create_table(parser).unwrap_err();

        assert!(matches!(err, RedshqlParseError::Malformed(_)));
    }
}

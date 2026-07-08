use sqlparser::{
    parser::{Parser, ParserError},
    tokenizer::{Token, Word},
};

use crate::statements::{copy::RedshqlCopy, create::RedshqlCreateTable};

pub mod copy;
pub mod create;

pub enum RedshiftStatement {
    Copy(RedshqlCopy),
    CreateTable(RedshqlCreateTable),
}

pub enum ParseResult {
    Match(RedshiftStatement),
    NoMatch,       // not a recognized Redshift-only command -> forward as-is
    Error(String), // recognized (e.g. starts with COPY) but malformed
}

#[derive(Debug)]
pub enum RedshqlParseError {
    NotACopyStatement,
    NotACreateStatement,
    Sql(ParserError),
    Malformed(String),
}

pub fn expect_word(parser: &mut Parser) -> Result<String, RedshqlParseError> {
    match parser.next_token().token {
        Token::Word(Word { value, .. }) => Ok(value),
        Token::SemiColon => Ok("SemiColon".into()),
        other => Err(RedshqlParseError::Malformed(format!(
            "expected identifier, got {other:?}"
        ))),
    }
}

pub fn parse_string_literal(parser: &mut Parser) -> Result<String, RedshqlParseError> {
    match parser.next_token().token {
        Token::SingleQuotedString(s) => Ok(s),
        other => Err(RedshqlParseError::Malformed(format!(
            "expected string literal, got {other:?}"
        ))),
    }
}

pub fn parse_number_literal(parser: &mut Parser) -> Result<u32, RedshqlParseError> {
    match parser.next_token().token {
        Token::Number(s, _) => s
            .parse()
            .map_err(|_| RedshqlParseError::Malformed(format!("bad number: {s}"))),
        other => Err(RedshqlParseError::Malformed(format!(
            "expected number, got {other:?}"
        ))),
    }
}

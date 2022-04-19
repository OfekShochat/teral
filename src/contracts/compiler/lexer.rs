use std::{collections::HashMap, str::FromStr};

use primitive_types::U256;
use thiserror::Error;

use crate::storage::{RocksdbStorage, Storage};

use crate::contracts::language::Opcode;

use super::CompileError;

#[derive(Debug, PartialEq, Clone)]
pub enum Base {
    Dec,
    Hex,
}

impl TryFrom<u32> for Base {
    type Error = CompileError;

    fn try_from(value: u32) -> Result<Self, CompileError> {
        match value {
            16 => Ok(Self::Hex),
            10 => Ok(Self::Dec),
            _ => Err(CompileError::BaseParse(value)),
        }
    }
}

impl Into<u32> for Base {
    fn into(self) -> u32 {
        match self {
            Self::Dec => 10,
            Self::Hex => 16,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum TokenKind {
    Keyword(Keyword),
    Type(Type),
    Num(Base, Type),
    Op(Bin),
    Ident,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Bin {
    Sub,
    Add,
    Mul,
    Div,
    Lt,
    Gt,
    Leq,
    Geq,
    EqSign,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    Mapping,
    Let,
    Peek,
    End,
    If,
    Else,
    Fnk,
    Get,
    Store,
    Dup,
    Require,
    In,
    Iszero,
}

impl TryFrom<&str> for Keyword {
    type Error = CompileError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "mapping" => Ok(Self::Mapping),
            "let" => Ok(Self::Let),
            "peek" => Ok(Self::Peek),
            "end" => Ok(Self::End),
            "if" => Ok(Self::If),
            "else" => Ok(Self::Else),
            "fn" => Ok(Self::Fnk),
            "get" => Ok(Self::Get),
            "store" => Ok(Self::Store),
            "dup" => Ok(Self::Dup),
            "require" => Ok(Self::Require),
            "in" => Ok(Self::In),
            "iszero" => Ok(Self::Iszero),
            _ => Err(CompileError::CantInterpret(
                value.to_string(),
                "keyword".to_string(),
            )),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Type {
    U256,
    U64,
    U32,
    U16,
    U8,
}

impl TryFrom<&str> for Type {
    type Error = CompileError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "u256" => Ok(Self::U256),
            "u64" => Ok(Self::U64),
            "u32" => Ok(Self::U32),
            "u16" => Ok(Self::U16),
            "u8" => Ok(Self::U8),
            _ => Err(CompileError::CantInterpret(
                value.to_string(),
                "type".to_string(),
            )),
        }
    }
}

impl Type {
    pub fn byte_count(&self) -> u8 {
        match self {
            Self::U256 => 32,
            Self::U64 => 8,
            Self::U32 => 4,
            Self::U16 => 2,
            Self::U8 => 1,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub value: String,
}

impl Token {
    fn new(kind: TokenKind, value: String) -> Self {
        Self { kind, value }
    }
}

pub struct Lexer {
    input: Vec<String>,
    index: usize,
}

impl Lexer {
    pub fn new(input: String) -> Self {
        Self {
            input: input.split_whitespace().map(String::from).collect(),
            index: 0,
        }
    }

    pub fn should_stop(&self) -> bool {
        self.index >= self.input.len()
    }

    fn curr(&self) -> &str {
        assert!(!self.should_stop());
        &self.input[self.index]
    }

    fn first(&self) -> char {
        assert!(!self.should_stop());
        self.input[self.index].chars().nth(0).unwrap()
    }

    fn second(&self) -> Result<char, CompileError> {
        if self.should_stop() {
            Err(CompileError::UnexpectedEow)
        } else {
            Ok(self.input[self.index]
                .chars()
                .nth(1)
                .ok_or(CompileError::UnexpectedEow)?)
        }
    }

    fn bump(&mut self) -> Result<&str, CompileError> {
        if self.should_stop() {
            Err(CompileError::ShouldStop)
        } else {
            self.index += 1;
            Ok(&self.input[self.index - 1])
        }
    }

    fn number(&mut self) -> Result<TokenKind, CompileError> {
        let (base, word) = if self.first() == '0' {
            match self.second() {
                Ok('x') => (16, &self.curr()[2..]),
                Ok('_') => (10, self.curr()),
                Err(_) => (10, self.curr()),
                _ => {
                    return Err(CompileError::CantInterpret(
                        self.curr().to_string(),
                        "num".to_string(),
                    ))
                }
            }
        } else {
            (10, self.curr())
        };

        let mut e = 0;
        for c in word.chars() {
            if !c.is_digit(base) {
                break;
            }
            e += 1;
        }

        let typ = if word.chars().nth(e) != Some('_') {
            Type::U256
        } else {
            Type::try_from(&word[e + 1..])?
        };

        Ok(TokenKind::Num(base.try_into()?, typ))
    }

    fn check_word(
        &self,
        word: &str,
        type_name: &str,
        predicate: impl Fn(char) -> bool,
    ) -> Result<usize, CompileError> {
        let mut index = 0;
        for c in word.chars() {
            if !predicate(c) {
                return Err(CompileError::CantInterpret(
                    word.to_string(),
                    type_name.to_string(),
                ));
            }
            index += 1;
        }
        Ok(index)
    }

    fn identifier(&mut self) -> Result<TokenKind, CompileError> {
        if let Ok(word) = Keyword::try_from(self.curr()) {
            return Ok(TokenKind::Keyword(word));
        }
        if let Ok(word) = Type::try_from(self.curr()) {
            return Ok(TokenKind::Type(word));
        }
        self.check_word(self.curr(), "identifier", |c| {
            c.is_alphanumeric() || c == '_'
        })?;
        Ok(TokenKind::Ident)
    }

    fn less_than(&self) -> Result<TokenKind, CompileError> {
        match self.second() {
            Ok('=') => Ok(TokenKind::Op(Bin::Leq)),
            Err(_) => Ok(TokenKind::Op(Bin::Lt)),
            _ => Err(CompileError::UnexpectedToken(
                self.second().unwrap().to_string(),
            )),
        }
    }

    fn more_than(&self) -> Result<TokenKind, CompileError> {
        match self.second() {
            Ok('=') => Ok(TokenKind::Op(Bin::Geq)),
            Err(_) => Ok(TokenKind::Op(Bin::Gt)),
            _ => Err(CompileError::UnexpectedToken(
                self.second().unwrap().to_string(),
            )),
        }
    }

    pub fn advance(&mut self) -> Result<Token, CompileError> {
        let kind = match self.first() {
            'a'..='z' | 'A'..='Z' | '_' => self.identifier()?,
            '0'..='9' => self.number()?,
            '=' if self.second()? == '=' => TokenKind::Op(Bin::EqSign),
            '-' => TokenKind::Op(Bin::Sub),
            '+' => TokenKind::Op(Bin::Add),
            '*' => TokenKind::Op(Bin::Mul),
            '/' => TokenKind::Op(Bin::Div),
            '<' => self.less_than()?,
            '>' => self.more_than()?,
            _ => {
                return Err(CompileError::CantInterpret(
                    self.curr().to_string(),
                    "any".to_string(),
                ))
            }
        };
        let value = match kind {
            TokenKind::Num(_, _) if self.curr().contains('_') => {
                let without_type = self.curr().split_once('_').unwrap().0;
                if self.curr().contains('x') {
                    without_type[2..].to_string()
                } else {
                    without_type.to_string()
                }
            }
            _ => self.curr().to_string(),
        };
        let tok = Token::new(kind, value);
        self.bump()?;
        Ok(tok)
    }
}
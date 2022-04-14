use std::{collections::HashMap, str::FromStr};

use primitive_types::U256;
use thiserror::Error;

use crate::storage::{RocksdbStorage, Storage};

use super::language::Opcode;

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("the compiler should have stopped but did not")]
    ShouldStop,
    #[error("unexpected end of word")]
    UnexpectedEow,
    #[error("unexpected end of code")]
    UnexpectedEoc,
    #[error("{0} syntax error: expected {1} got {2}")]
    SyntaxError(usize, String, String),
    #[error("'{0}' was unexpected in this context")]
    UnexpectedToken(String),
    #[error("can not interpret {0} as a {1}")]
    CantInterpret(String, String),
    #[error("could not convert {0} to Base")]
    BaseParse(u32),
    #[error("eventually expected `{0}` but got <eof>")]
    EventuallyExpected(String),
}

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
    Ident,
    EqSign,
    Sub,
    Add,
    Mul,
    Div,
    Lt,
    Gt,
    Leq,
    Geq,
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
    fn byte_count(&self) -> u8 {
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
    kind: TokenKind,
    value: String,
}

impl Token {
    fn new(kind: TokenKind, value: String) -> Self {
        Self { kind, value }
    }
}

struct Lexer {
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

    fn should_stop(&self) -> bool {
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
        let (base, word) = if self.first() == '0' && self.second()? == 'x' {
            (16, &self.curr()[2..])
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
            Ok('=') => Ok(TokenKind::Leq),
            Err(_) => Ok(TokenKind::Geq),
            _ => Err(CompileError::UnexpectedToken(
                self.second().unwrap().to_string(),
            )),
        }
    }

    fn more_than(&self) -> Result<TokenKind, CompileError> {
        match self.second() {
            Ok('=') => Ok(TokenKind::Geq),
            Err(_) => Ok(TokenKind::Gt),
            _ => Err(CompileError::UnexpectedToken(
                self.second().unwrap().to_string(),
            )),
        }
    }

    fn advance(&mut self) -> Result<Token, CompileError> {
        let kind = match self.first() {
            'a'..='z' | 'A'..='Z' | '_' => self.identifier()?,
            '0'..='9' => self.number()?,
            '=' => TokenKind::EqSign,
            '-' => TokenKind::Sub,
            '+' => TokenKind::Add,
            '*' => TokenKind::Mul,
            '/' => TokenKind::Div,
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

#[derive(Debug)]
struct Compiler {
    input: Vec<Token>,
    index: usize,
    functions: HashMap<String, (usize, Vec<String>)>,
    output: Vec<u8>,
    binded_context: Vec<String>,
}

impl Compiler {
    fn new(input: Vec<Token>) -> Self {
        Self {
            input,
            index: 0,
            functions: HashMap::new(),
            output: vec![],
            binded_context: vec![],
        }
    }

    fn should_stop(&self) -> bool {
        self.index >= self.input.len()
    }

    fn bump(&mut self) -> Result<&Token, CompileError> {
        if self.should_stop() {
            Err(CompileError::ShouldStop)
        } else {
            self.index += 1;
            Ok(&self.input[self.index - 1])
        }
    }

    fn first(&self) -> &Token {
        &self.input[self.index]
    }

    fn second(&self) -> Result<&Token, CompileError> {
        if self.index + 1 >= self.input.len() {
            Err(CompileError::UnexpectedEoc)
        } else {
            Ok(&self.input[self.index + 1])
        }
    }

    fn get_parameters(&mut self) -> Result<Vec<String>, CompileError> {
        let mut parameters = vec![];
        loop {
            self.bump()?;
            if self.should_stop() {
                return Err(CompileError::EventuallyExpected("in".to_string()));
            }
            if self.first().kind == TokenKind::Keyword(Keyword::In) {
                break;
            }
            parameters.push(self.first().value.clone());
        }
        self.bump()?;
        Ok(parameters)
    }

    fn function(&mut self) -> Result<(), CompileError> {
        let name = self.bump()?.value.clone();
        let parameters = self.get_parameters()?;
        self.functions.insert(name, (self.output.len(), parameters));

        self.advance_until_end()?;
        Ok(())
    }

    fn number(&mut self, base: Base, typ: Type) -> Result<(), CompileError> {
        let num = &self.first().value.clone();
        self.push_opcode(Opcode::Push(typ.byte_count()));
        match typ {
            Type::U256 => {
                let bytes = &mut [0; 32];
                U256::from_str_radix(&num, base.into())
                    .map_err(|_| CompileError::CantInterpret(num.to_string(), "u256".to_string()))?
                    .to_little_endian(bytes);
                self.output.append(&mut bytes.to_vec());
            }
            Type::U64 => self.output.append(
                &mut u64::from_str_radix(&num, base.into())
                    .map_err(|_| CompileError::CantInterpret(num.to_string(), "u64".to_string()))?
                    .to_le_bytes()
                    .to_vec(),
            ),
            Type::U32 => self.output.append(
                &mut u32::from_str_radix(&num, base.into())
                    .map_err(|_| CompileError::CantInterpret(num.to_string(), "u32".to_string()))?
                    .to_le_bytes()
                    .to_vec(),
            ),
            Type::U16 => self.output.append(
                &mut u16::from_str_radix(&num, base.into())
                    .map_err(|_| CompileError::CantInterpret(num.to_string(), "u16".to_string()))?
                    .to_le_bytes()
                    .to_vec(),
            ),
            Type::U8 => self.output.append(
                &mut u8::from_str_radix(&num, base.into())
                    .map_err(|_| CompileError::CantInterpret(num.to_string(), "u8".to_string()))?
                    .to_le_bytes()
                    .to_vec(),
            ),
        }; // can simplify this..
        self.bump()?;
        Ok(())
    }

    fn bind_block(&mut self, pop: bool) -> Result<(), CompileError> {
        let names = self.get_parameters()?;
        if pop {
            self.push_opcode(Opcode::MoveToReturn(names.len().try_into().unwrap()));
        } else {
            self.push_opcode(Opcode::CopyToReturn(names.len().try_into().unwrap()));
        }
        self.binded_context = names;

        self.advance_until_end()?;
        self.push_opcode(Opcode::ClearReturn);
        Ok(())
    }

    fn identifier(&mut self) -> Result<(), CompileError> {
        if self.binded_context.contains(&self.first().value) {
            let pos = self
                .binded_context
                .iter()
                .position(|x| *x == self.first().value);
            self.push_opcode(Opcode::CopyToMain(pos.unwrap() as u8));
            self.bump()?;
            Ok(())
        } else {
            Err(CompileError::UnexpectedToken("identifier".to_string()))
        }
    }

    fn if_(&mut self) -> Result<(), CompileError> {
        let before = self.index;
        self.bump()?;
        self.advance_while(|k| {
            k != TokenKind::Keyword(Keyword::Else) && k != TokenKind::Keyword(Keyword::End)
        })?;
        let with_else = self.input[self.index - 1].kind == TokenKind::Keyword(Keyword::Else);
        self.push_opcode(Opcode::Push(1));
        self.output.push((self.index - before) as u8);
        self.push_opcode(Opcode::Jumpif);
        if with_else {
            self.push_opcode(Opcode::Push(1));
            let before = self.output.len();
            self.push_opcode(Opcode::Jump);
            self.advance_until_end()?;
            self.output.insert(before, (self.output.len() - before - 1) as u8);
        }
        Ok(())
    }

    fn advance_until_end(&mut self) -> Result<(), CompileError> {
        self.advance_while(|k| k != TokenKind::Keyword(Keyword::End))
    }

    fn advance_while(&mut self, predicate: impl Fn(TokenKind) -> bool) -> Result<(), CompileError> {
        if self.should_stop() {
            return Err(CompileError::UnexpectedEoc);
        }
        while predicate(self.first().kind.clone()) {
            self.advance()?;
            if self.should_stop() {
                return Err(CompileError::UnexpectedEoc);
            }
        }
        self.bump()?;
        Ok(())
    }

    fn push_opcode(&mut self, opcode: Opcode) {
        self.output.push(opcode.to_u8());
    }

    fn advance(&mut self) -> Result<(), CompileError> {
        match self.first().kind.clone() {
            TokenKind::Num(base, typ) => self.number(base, typ)?,
            TokenKind::Keyword(Keyword::Let) => self.bind_block(true)?,
            TokenKind::Keyword(Keyword::Peek) => self.bind_block(false)?,
            TokenKind::Keyword(Keyword::Fnk) => self.function()?,
            TokenKind::Keyword(Keyword::If) => self.if_()?,
            TokenKind::Ident => self.identifier()?,
            _ => panic!("{:?}", self.first().kind),
        }
        Ok(())
    }
}

pub fn parse(input: String) {
    println!("\n\n");
    let st = std::time::Instant::now();
    let input = lex(r#"
fn transfer from to amount in
    0x29d7d1dd5b6f9c864d9db560d72a247c178ae86b
    let poopoo in
        poopoo
        if
        else
        end
    end
end"#.to_string());
    let mut compiler = Compiler::new(input);
    compiler.advance().unwrap();
    println!("{:?}", st.elapsed());
    println!("{:?} {:?}", compiler.functions, compiler.output.len());
    println!("{:?}", somewhat_decompile(&compiler.output));
    super::execute(
        compiler.output.clone(),
        vec![],
        RocksdbStorage::load(&Default::default()),
    );
    println!("\n\n");
}

fn somewhat_decompile(input: &[u8]) -> Vec<Opcode> {
    let mut out = vec![];
    let mut i = 0;
    while i < input.len() {
        if let Some(poop) = Opcode::from_u8(input[i]) {
            match poop {
                Opcode::Push(n) => i += n as usize,
                _ => {}
            }
            out.push(poop);
        }
        i += 1;
    }
    out
}

pub fn lex(input: String) -> Vec<Token> {
    // let input = r#"
    // mapping Balances
    // fn transfer u256 from u256 to u64 amount
    // Balances from get
    // peek from_balance
    // require
    // from_balance amount > require

    // Balances
    // from
    // from_balance amount -
    // store

    // Balances to get
    // let to_balance if
    //     Balances to
    //     to_balance amount +
    //     store
    // else
    //     Balances
    // to amount
    // store
    // end
    // end
    // "#.to_string();
    let mut lexer = Lexer::new(input);
    let mut tokens = vec![];
    while !lexer.should_stop() {
        let possible = lexer.advance();
        if let Err(err) = possible {
            eprintln!("{}", err);
            break;
        }
        tokens.push(possible.unwrap());
    }
    // println!("{:?}", tokens);
    tokens
}

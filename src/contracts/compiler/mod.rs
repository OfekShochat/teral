mod lexer;
mod tests;

use std::{collections::HashMap, str::FromStr};

use primitive_types::U256;
use thiserror::Error;

use crate::storage::{RocksdbStorage, Storage};

use lexer::{Token, TokenKind, Bin, Keyword, Lexer, Base, Type};

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
        self.bump()?;
        let name = self.first().value.clone();
        let mut parameters = self.get_parameters()?;
        self.functions
            .insert(name, (self.output.len(), parameters.clone()));

        self.binded_context.append(&mut parameters);
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
        let names = &mut self.get_parameters()?;
        if pop {
            self.push_opcode(Opcode::MoveToReturn(names.len().try_into().unwrap()));
        } else {
            self.push_opcode(Opcode::CopyToReturn(names.len().try_into().unwrap()));
        }
        self.binded_context.append(names);

        self.advance_until_end()?;
        // self.push_opcode(Opcode::ClearReturn); // TODO: clear only the ones we added rn.
        self.binded_context.truncate(names.len());
        Ok(())
    }

    fn identifier(&mut self) -> Result<(), CompileError> {
        if self.binded_context.contains(&self.first().value) {
            let pos = self
                .binded_context
                .iter()
                .rev() // if we push anything with the same name, we want to get the latest one
                .position(|x| *x == self.first().value);
            self.push_opcode(Opcode::CopyToMain(
                (self.binded_context.len() - pos.unwrap() - 1) as u8,
            ));
            self.bump()?;
            Ok(())
        } else {
            Err(CompileError::UnexpectedToken(self.first().value.clone()))
        }
    }

    fn if_(&mut self) -> Result<(), CompileError> {
        self.bump()?;
        let to = self.input[self.index..]
            .iter()
            .position(|tok| {
                tok.kind == TokenKind::Keyword(Keyword::Else)
                    || tok.kind == TokenKind::Keyword(Keyword::End)
            })
            .expect("Could not find else/end keywords to end `if`");
        self.push_opcode(Opcode::Push(1));
        let before = self.output.len();
        self.push_opcode(Opcode::Jumpif);

        self.advance_while(|k| {
            k != TokenKind::Keyword(Keyword::Else) && k != TokenKind::Keyword(Keyword::End)
        })?;

        let with_else = self.input[self.index - 1].kind == TokenKind::Keyword(Keyword::Else);
        if with_else {
            self.output
                .insert(before, (self.output.len() - before + 2) as u8);
            self.push_opcode(Opcode::Push(1));
            let before = self.output.len();
            self.push_opcode(Opcode::Jump);
            self.advance_until_end()?;
            self.output
                .insert(before, (self.output.len() - before - 1) as u8);
        } else {
            self.output
                .insert(before, (self.output.len() - before - 1) as u8);
        }
        Ok(())
    }

    fn op(&mut self, op: Bin) -> Result<(), CompileError> {
        let kind = match op {
            Bin::Sub => Opcode::Sub,
            Bin::Add => Opcode::Add,
            Bin::Mul => Opcode::Mul,
            Bin::Div => Opcode::Div,
            Bin::Lt => Opcode::Lt,
            Bin::Gt => Opcode::Gt,
            Bin::Geq => Opcode::Geq,
            Bin::Leq => Opcode::Geq,
            Bin::EqSign => Opcode::Eqi,
        };
        self.push_opcode(kind);
        self.bump()?;
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
            self.advance_within_function()?;
            if self.should_stop() {
                return Err(CompileError::UnexpectedEoc);
            }
        }
        self.bump()?;
        Ok(())
    }

    fn require(&mut self) -> Result<(), CompileError> {
        self.push_opcode(Opcode::Push(1));
        self.output.push(1);
        self.push_opcode(Opcode::Jumpifnot);
        self.push_opcode(Opcode::Terminate);
        self.bump()?;
        Ok(())
    }

    fn push_opcode(&mut self, opcode: Opcode) {
        self.output.push(opcode.to_u8());
    }

    fn advance_within_function(&mut self) -> Result<(), CompileError> {
        match self.first().kind.clone() {
            TokenKind::Num(base, typ) => self.number(base, typ)?,
            TokenKind::Keyword(Keyword::Let) => self.bind_block(true)?,
            TokenKind::Keyword(Keyword::Peek) => self.bind_block(false)?,
            TokenKind::Keyword(Keyword::If) => self.if_()?,
            TokenKind::Keyword(Keyword::Require) => self.require()?,
            TokenKind::Ident => self.identifier()?,
            TokenKind::Keyword(Keyword::Iszero) => {
                self.push_opcode(Opcode::Iszero);
                self.bump()?;
            }
            TokenKind::Keyword(Keyword::Get) => {
                self.push_opcode(Opcode::Get);
                self.bump()?;
            }
            TokenKind::Keyword(Keyword::Store) => {
                self.push_opcode(Opcode::Store);
                self.bump()?;
            }
            TokenKind::Op(op) => self.op(op)?,
            _ => panic!("{:?}", self.first().kind),
        }
        Ok(())
    }

    fn advance(&mut self) -> Result<(), CompileError> {
        match self.first().kind.clone() {
            TokenKind::Keyword(Keyword::Fnk) => self.function()?,
            TokenKind::Keyword(Keyword::Mapping) => {
                if self.second()?.kind != TokenKind::Ident {
                    return Err(CompileError::UnexpectedToken(self.second()?.value.clone()));
                }
                self.binded_context.push(self.second()?.value.clone());
                self.bump()?;
                self.bump()?;
            }
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
    0_u8
    if
        10
    end

    from amount +
    if
        20
    end
end"#
        .to_string());
    let mut compiler = Compiler::new(input);
    compiler.advance().unwrap();
    println!("{:?}", st.elapsed());
    println!("{:?} {:?}", compiler.functions, compiler.output.len());
    println!("{:?}", somewhat_decompile(&compiler.output));
    super::execute(
        compiler.output.clone(),
        vec![U256::from(1234), U256::from(1235), U256::from(101)],
        RocksdbStorage::load(&Default::default()),
    );
    println!("\n\n");
}

fn somewhat_decompile(input: &[u8]) -> Vec<(Opcode, U256)> {
    let mut out = vec![];
    let mut i = 0;
    while i < input.len() {
        if let Some(poop) = Opcode::from_u8(input[i]) {
            let a = match poop {
                Opcode::Push(n) => {
                    i += n as usize;
                    (
                        poop,
                        U256::from_little_endian(&input[i - n as usize + 1..i as usize + 1]),
                    )
                    // (poop, U256::from(0))
                }
                _ => (poop, U256::from(0_usize)),
            };
            out.push(a);
        }
        i += 1;
    }
    out
}

pub fn lex(input: String) -> Vec<Token> {
    let mut lexer = Lexer::new(input);
    let mut tokens = vec![];
    while !lexer.should_stop() {
        let possible = lexer.advance();
        tokens.push(possible.unwrap());
    }
    // println!("{:?}", tokens);
    tokens
}

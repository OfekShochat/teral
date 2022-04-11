use std::{fmt, sync::Arc};

use primitive_types::U256;
use sha3::Digest;
use thiserror::Error;

use crate::storage::Storage;

const STACK_SIZE: usize = 32;

#[derive(Debug, Error)]
enum VmError {
    #[error("the code should have stopped (possible reasons are: invalid opcode, reached end of code, or the program raised Stop)")]
    ShouldStop,
    #[error("stack underflow")]
    StackUnderflow,
    #[error("stack overflow")]
    StackOverflow,
    #[error("expected to be at least 32 bytes left but there are only {0} left")]
    ExpectedValue(usize),
    #[error("tried to jump to {0} but the code's length is only {1}")]
    InvalidJump(U256, usize),
}

#[derive(Debug)]
enum Opcode {
    Terminate,
    Add,
    Sub,
    Mul,
    Div,
    Store,
    Get,
    Push(u8),
    Swap(u8),
    Jumpif,
    Jump,
}

impl Opcode {
    fn from_u8(opcode: u8) -> Option<Self> {
        match opcode {
            0x00 => Some(Self::Terminate),
            0x01 => Some(Self::Add),
            0x02 => Some(Self::Sub),
            0x03 => Some(Self::Mul),
            0x04 => Some(Self::Div),
            0x05 => Some(Self::Store),
            0x06 => Some(Self::Get),
            0x07..=0x26 => Some(Self::Push(opcode - 0x06)),
            0x27..=0x47 => Some(Self::Swap(opcode - 0x07)),
            0x48 => Some(Self::Jumpif),
            0x49 => Some(Self::Jump),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct Stack {
    stack: [U256; STACK_SIZE],
    stack_pos: usize,
}

impl Stack {
    fn new() -> Self {
        Self {
            stack: [U256::zero(); STACK_SIZE],
            stack_pos: 1,
        }
    }

    fn push_multiple(&mut self, values: Vec<U256>) -> Result<(), VmError> {
        for v in values {
            self.push(v)?;
        }
        Ok(())
    }

    fn pop(&mut self) -> Result<U256, VmError> {
        if self.stack_pos == 1 {
            return Err(VmError::StackUnderflow);
        }
        self.stack_pos -= 1;
        let ret = Ok(self.stack[self.stack_pos - 1]);
        self.stack[self.stack_pos - 1] = U256::zero();
        ret
    }

    fn push(&mut self, value: U256) -> Result<(), VmError> {
        if self.stack_pos > STACK_SIZE {
            Err(VmError::StackOverflow)
        } else {
            self.stack[self.stack_pos - 1] = value;
            self.stack_pos += 1;
            Ok(())
        }
    }

    fn swap(&mut self, nth: u8) -> Result<(), VmError> {
        assert!(nth <= self.stack.len() as u8);
        self.stack.swap(self.stack_pos - 1, nth as usize - 1);
        Ok(())
    }
}

impl fmt::Debug for Vm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vm")
            .field("opcodes", &self.opcodes)
            .field("pc", &self.index)
            .field("stack", &self.stack)
            .field("should_stop", &self.should_stop())
            .field("terminated", &self.terminated)
            .field("stores", &self.stores)
            .finish()
    }
}

struct Vm {
    stack: Stack,
    opcodes: Vec<u8>,
    index: usize,
    storage: Arc<dyn Storage>,
    terminated: bool,
    stores: Vec<(U256, U256)>,
    contract_hash: [u8; 32],
}

impl Vm {
    fn new(
        contract_hash: [u8; 32],
        opcodes: Vec<u8>,
        storage: Arc<dyn Storage>,
    ) -> Result<Self, VmError> {
        Ok(Self {
            stack: Stack::new(),
            opcodes,
            index: 0,
            storage,
            terminated: false,
            stores: vec![],
            contract_hash,
        })
    }

    fn with_arguments(
        contract_hash: [u8; 32],
        opcodes: Vec<u8>,
        args: Vec<U256>,
        storage: Arc<dyn Storage>,
    ) -> Result<Self, VmError> {
        let mut stack = Stack::new();
        stack.push_multiple(args)?;

        Ok(Self {
            stack,
            opcodes,
            index: 0,
            storage,
            terminated: false,
            stores: vec![],
            contract_hash,
            // somehow designate a storage location to this storage with this account. maybe hash
            // the two together?
        })
    }

    fn next(&mut self) -> Option<Opcode> {
        if self.should_stop() {
            return None;
        }
        self.index += 1;
        Opcode::from_u8(self.opcodes[self.index - 1])
    }

    fn should_stop(&self) -> bool {
        self.terminated || self.index >= self.opcodes.len()
    }

    fn advance(&mut self) -> Result<(), VmError> {
        let op = self.next().ok_or(VmError::ShouldStop)?;

        match op {
            Opcode::Terminate => self.terminated = true,
            Opcode::Add => {
                let lhs = self.stack.pop()?;
                let rhs = self.stack.pop()?;
                self.stack.push(lhs + rhs)?;
            }
            Opcode::Sub => {
                let lhs = self.stack.pop()?;
                let rhs = self.stack.pop()?;
                self.stack.push(lhs - rhs)?;
            }
            Opcode::Mul => {
                let lhs = self.stack.pop()?;
                let rhs = self.stack.pop()?;
                self.stack.push(lhs * rhs)?;
            }
            Opcode::Div => {
                let lhs = self.stack.pop()?;
                let rhs = self.stack.pop()?;
                if rhs.is_zero() {
                    self.stack.push(U256::zero())?;
                } else {
                    self.stack.push(lhs / rhs)?;
                }
            }
            Opcode::Store => {
                let value = self.stack.pop()?;
                let key = self.stack.pop()?;
                self.stores.push((key, value));
            }
            Opcode::Get => {
                let key = self.stack.pop()?;
                if let Some(value) = self.get_from_storage(1, key) {
                    self.stack.push(value)?;
                } else {
                    self.stack.push(U256::zero())?;
                }
            }
            Opcode::Push(n) => {
                self.index += n as usize;
                if self.index > self.opcodes.len() {
                    return Err(VmError::ExpectedValue(self.index - self.opcodes.len()));
                }
                let value =
                    U256::from_little_endian(&self.opcodes[self.index - n as usize..self.index]);
                self.stack.push(value)?;
            }
            Opcode::Swap(n) => self.stack.swap(n)?,
            Opcode::Jumpif => {
                let cond = self.stack.pop()?;
                let alternative_offset = self.stack.pop()?;
                if cond == U256::zero() {
                    if alternative_offset < U256::from(self.opcodes.len() - self.index) {
                        self.index += alternative_offset.as_usize() - 1;
                    } else {
                        return Err(VmError::InvalidJump(alternative_offset + U256::from(self.index), self.opcodes.len()));
                    }
                }
            }
            Opcode::Jump => {
                let alternative = self.stack.pop()?;
                if alternative < U256::from(self.opcodes.len() - self.index) {
                    self.index += alternative.as_usize() - 1;
                } else {
                    return Err(VmError::InvalidJump(alternative, self.opcodes.len()));
                }
            }
        }
        Ok(())
    }

    fn get_from_storage(&self, map_index: usize, key: U256) -> Option<U256> {
        let mut key_bytes = [0; 32];
        key.to_little_endian(&mut key_bytes);

        let mut hasher = sha3::Sha3_256::new();
        hasher.update(map_index.to_le_bytes());
        hasher.update(key_bytes);
        hasher.update(self.contract_hash);
        Some(U256::from_little_endian(
            &self.storage.get(&hasher.finalize())?,
        ))
    }
}

pub fn execute(_opcodes: Vec<u8>, args: Vec<U256>, storage: Arc<dyn Storage>) {
    let opcodes = vec![0x48, 0x00, 0x07, 4];
    let st = std::time::Instant::now();
    let mut vm =
        Vm::with_arguments([0; 32], opcodes, vec![U256::from(2), U256::from(0)], storage).unwrap();
    while !vm.should_stop() {
        vm.advance().unwrap();
    }
    let end = st.elapsed();
    println!("{:?}", end);
    println!("{:?}", 1.0 / (end.as_secs_f64() * 3.0));
    tracing::info!("{:?}", vm);
}

#[cfg(test)]
mod tests {}

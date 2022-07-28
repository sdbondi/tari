//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{cell::Cell, sync::Arc};

use tari_template_abi::{FunctionDef, LogLevel, TemplateDef};
use wasmer::{Extern, Function, Instance, Memory, Module, Store, Val, WasmerEnv};

use crate::{
    models::{Component, ComponentId},
    package::PackageModuleLoader,
    runtime::{Runtime, RuntimeError, RuntimeInterface},
    wasm::env::WasmEnv,
};

#[derive(Debug, Clone)]
pub struct WasmModule {
    code: Vec<u8>,
}

impl WasmModule {
    pub fn from_code(code: Vec<u8>) -> Self {
        Self { code }
    }

    pub fn code(&self) -> &[u8] {
        &self.code
    }
}

impl PackageModuleLoader for WasmModule {
    type Error = WasmModuleError;
    type Loaded = LoadedWasmModule;

    fn load_module(&self) -> Result<Self::Loaded, Self::Error> {
        let store = Store::default();
        let module = Module::new(&store, &self.code)?;
        let runtime = Runtime::new(Arc::new(NoopInterface));
        let mut env = WasmEnv::new(runtime);

        fn stub(_env: &WasmEnv, _op: i32, _arg_ptr: i32, _arg_len: i32) -> i32 {
            panic!("WASM module called engine while loading ABI")
        }

        let stub = Function::new_native_with_env(&store, env.clone(), stub);
        let imports = env.create_resolver(&store, stub);
        let instance = Instance::new(&module, &imports)?;
        env.init_with_instance(&instance)?;
        validate_environment(&env)?;

        let template = initialize_and_load_template_abi(&instance)?;
        Ok(LoadedWasmModule::new(template, module))
    }
}

#[derive(Debug, Clone)]
pub struct LoadedWasmModule {
    template: TemplateDef,
    module: wasmer::Module,
}

impl LoadedWasmModule {
    pub fn new(template: TemplateDef, module: wasmer::Module) -> Self {
        Self { template, module }
    }

    pub fn wasm_module(&self) -> &wasmer::Module {
        &self.module
    }

    pub fn template_name(&self) -> &str {
        &self.template.template_name
    }

    pub fn find_func_by_name(&self, function_name: &str) -> Option<&FunctionDef> {
        self.template.functions.iter().find(|f| f.name == *function_name)
    }
}

fn initialize_and_load_template_abi(instance: &Instance) -> Result<TemplateDef, WasmModuleError> {
    let abi_func = instance
        .exports
        .iter()
        .find_map(|(name, export)| match export {
            Extern::Function(f) if name.ends_with("_abi") => Some(f),
            _ => None,
        })
        .ok_or(WasmModuleError::NoAbiDefinition)?;

    // Initialize ABI memory
    let ret = abi_func.call(&[])?;
    let ptr = match ret.get(0) {
        Some(Val::I32(ptr)) => *ptr as u32,
        Some(_) | None => return Err(WasmModuleError::InvalidReturnTypeFromAbiFunc),
    };

    // Load ABI from memory
    let memory = instance.exports.get_memory("memory")?;
    let data = copy_abi_data_from_memory_checked(memory, ptr)?;
    let decoded = tari_template_abi::decode(&data).map_err(|_| WasmModuleError::AbiDecodeError)?;
    Ok(decoded)
}

fn copy_abi_data_from_memory_checked(memory: &Memory, ptr: u32) -> Result<Vec<u8>, WasmModuleError> {
    // Check memory bounds
    if memory.data_size() < u64::from(ptr) {
        return Err(WasmModuleError::AbiPointerOutOfBounds);
    }

    let view = memory.uint8view().subarray(ptr, memory.data_size() as u32 - 1);
    let data = &*view;
    if data.len() < 4 {
        return Err(WasmModuleError::MemoryUnderflow {
            required: 4,
            remaining: data.len(),
        });
    }

    fn copy_from_cell_slice(src: &[Cell<u8>], dest: &mut [u8], len: usize) {
        for i in 0..len {
            dest[i] = src[i].get();
        }
    }

    let mut buf = [0u8; 4];
    copy_from_cell_slice(data, &mut buf, 4);
    let len = u32::from_le_bytes(buf) as usize;
    const MAX_ABI_DATA_LEN: usize = 1024 * 1024;
    if len > MAX_ABI_DATA_LEN {
        return Err(WasmModuleError::AbiDataTooLarge {
            max: MAX_ABI_DATA_LEN,
            size: len,
        });
    }
    if data.len() < 4 + len {
        return Err(WasmModuleError::MemoryUnderflow {
            required: 4 + len,
            remaining: data.len(),
        });
    }

    let mut data = vec![0u8; len];
    let src = view.subarray(4, 4 + len as u32);
    copy_from_cell_slice(&*src, &mut data, len);
    Ok(data)
}

pub fn validate_environment(env: &WasmEnv) -> Result<(), WasmModuleError> {
    const MAX_MEM_SIZE: usize = 2 * 1024 * 1024;
    let mem_size = env.mem_size();
    if mem_size.bytes().0 > MAX_MEM_SIZE {
        return Err(WasmModuleError::MaxMemorySizeExceeded);
    }
    // TODO other package validations

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum WasmModuleError {
    #[error(transparent)]
    CompileError(#[from] wasmer::CompileError),
    #[error(transparent)]
    InstantiationError(#[from] wasmer::InstantiationError),
    #[error(transparent)]
    RuntimeError(#[from] wasmer::RuntimeError),
    #[error(transparent)]
    ExportError(#[from] wasmer::ExportError),
    #[error(transparent)]
    HostEnvInitError(#[from] wasmer::HostEnvInitError),
    #[error("Failed to decode ABI")]
    AbiDecodeError,
    #[error("maximum module memory size exceeded")]
    MaxMemorySizeExceeded,
    #[error("package did not contain an ABI definition")]
    NoAbiDefinition,
    #[error("package ABI function returned an invalid type")]
    InvalidReturnTypeFromAbiFunc,
    #[error("package ABI function returned an out of bounds pointer")]
    AbiPointerOutOfBounds,
    #[error("memory underflow: {required} bytes required but {remaining} remaining")]
    MemoryUnderflow { required: usize, remaining: usize },
    #[error("ABI data is too large: a maximum of {max} bytes allowed but size is {size}")]
    AbiDataTooLarge { max: usize, size: usize },
}

struct NoopInterface;

impl RuntimeInterface for NoopInterface {
    fn emit_log(&self, _level: LogLevel, _message: &str) {}

    fn create_component(&self, _component: Component) -> Result<ComponentId, RuntimeError> {
        panic!("create_component called on NoopInterface")
    }
}

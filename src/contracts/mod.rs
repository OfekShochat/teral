#[cfg(tests)]
mod tests;

use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;
use std::sync::{Mutex, Arc};
use wasmer::WasmerEnv;

use wasmer::{Function, imports, wat2wasm, Instance, Module, Store, Value, Singlepass, Universal};

pub fn poop() -> Result<(), Box<dyn std::error::Error>> {
    let wasm_bytes = wat2wasm(
        br#"
(module
  (func $get_counter (import "env" "get_counter") (result i32))
  (func $add_to_counter (import "env" "add_to_counter") (param i32) (result i32))
  (type $increment_t (func (param i32) (result i32)))
  (func $increment_f (type $increment_t) (param $x i32) (result i32)
    (block
      (loop
        (call $add_to_counter (i32.const 1))
        (set_local $x (i32.sub (get_local $x) (i32.const 1)))
        (br_if 1 (i32.eq (get_local $x) (i32.const 0)))
        (br 0)))
    call $get_counter)
  (export "increment_counter_loop" (func $increment_f)))
"#,
    )?;
    let now = Instant::now();
    let engine = Universal::new(Singlepass::default()).engine();
    let store = Store::new(&engine);
    let module = Module::new(&store, wasm_bytes)?;

    let shared_counter = Arc::new(AtomicI32::new(0));
    #[derive(WasmerEnv, Clone)]
    struct Env {
        counter: Arc<AtomicI32>,
    }

    // Create the functions
    fn get_counter(env: &Env) -> i32 {
        env.counter.load(Ordering::SeqCst)
    }
    fn add_to_counter(env: &Env, add: i32) -> i32 {
        env.counter.fetch_add(add, Ordering::SeqCst)
    }
    let import_object = imports! {
        "env" => {
            "get_counter" => Function::new_native_with_env(&store, Env { counter: shared_counter.clone() }, get_counter),
            "add_to_counter" => Function::new_native_with_env(&store, Env { counter: shared_counter.clone() }, add_to_counter),
        }
    };
    let instance = Instance::new(&module, &import_object)?;
    println!("{:?}", now.elapsed());
let increment_counter_loop = instance
        .exports
        .get_function("increment_counter_loop")?
        .native::<i32, i32>()?;

    println!("Initial ounter value: {:?}", shared_counter.load(Ordering::SeqCst));

    println!("Calling `increment_counter_loop` function...");
    // Let's call the `increment_counter_loop` exported function.
    //
    // It will loop five times thus incrementing our counter five times.
    let result = increment_counter_loop.call(5)?;

    println!("New counter value (host): {:?}", shared_counter.load(Ordering::SeqCst));

    println!("New counter value (guest): {:?}", result);
    Ok(())
}
